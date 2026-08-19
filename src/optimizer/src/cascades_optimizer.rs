use std::{cell::RefCell, rc::Rc};

use common::{
    catalog::CatalogRef, physical_expr::physical_rel_expr::PhysicalRelExpr,
    query::query_registrar::QueryStateRegistrar,
};
use queryexe::{query::translate_and_validate::Query, Managers};

use crate::cost::cardinality_cost_model::PlanCostEstimator;
use crate::memo::Memo;

/// A real, cost-driven optimizer: same `optimize(&Query, ...) -> PhysicalRelExpr`
/// interface as `MockOptimizer` (drop-in compatible), but instead of a
/// straight structural translation, it builds a `Memo` and picks each
/// subplan's physical form by estimated cost — see `memo.rs` for exactly
/// what "picks" means here (join algorithm, and index-scan vs. full-scan).
pub struct CascadesOptimizer<C: PlanCostEstimator> {
    cost_model: Rc<RefCell<C>>,
    #[allow(dead_code)]
    managers: &'static Managers,
    /// Needed (beyond what `Managers` carries) to look up which indexes
    /// exist on a table when weighing an `IndexScan` against a full scan.
    catalog: CatalogRef,
}

impl<C: PlanCostEstimator + 'static> CascadesOptimizer<C> {
    pub fn new(cost_model: C, managers: &'static Managers, catalog: CatalogRef) -> Self {
        Self {
            cost_model: Rc::new(RefCell::new(cost_model)),
            managers,
            catalog,
        }
    }

    pub fn optimize(
        &self,
        query: &Query,
        _query_registrar: Option<&'static QueryStateRegistrar>,
    ) -> PhysicalRelExpr {
        let memo: Memo<C::Cost> = Memo::new();
        let root = memo.add_group_from_plan_with_catalog(query.get_plan(), &self.catalog);
        let cost_model = self.cost_model.borrow();
        let (plan, _cost) = memo.resolve_costed(root, &*cost_model, query.get_environment());
        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::catalog::Catalog;
    use common::ids::{StateType, TransactionId};
    use common::physical::col_id_generator::ColIdGenerator;
    use common::physical::config::ServerConfig;
    use common::physical::small_string::StringManager;
    use common::query::rules::Rules;
    use common::table::{IndexInfo, TableInfo};
    use common::traits::stat_manager_trait::StatManagerTrait;
    use common::traits::storage_trait::StorageTrait;
    use common::{DataType, Field, TableSchema, Tuple};
    use queryexe::query::Translator;
    use queryexe::stats::reservoir_stat_manager::ReservoirStatManager;
    use queryexe::{IndexManager, StorageManager};
    use std::sync::Arc;

    use crate::cost::cardinality_cost_model::CardinalityCostModel;
    use crate::testutil;

    fn contains<F: Fn(&PhysicalRelExpr) -> bool + Copy>(p: &PhysicalRelExpr, pred: F) -> bool {
        if pred(p) {
            return true;
        }
        match p {
            PhysicalRelExpr::Select { src, .. }
            | PhysicalRelExpr::Project { src, .. }
            | PhysicalRelExpr::Sort { src, .. }
            | PhysicalRelExpr::HashAggregate { src, .. }
            | PhysicalRelExpr::Map { input: src, .. }
            | PhysicalRelExpr::FlatMap { input: src, .. }
            | PhysicalRelExpr::Rename { src, .. }
            | PhysicalRelExpr::Delete { src, .. }
            | PhysicalRelExpr::Update { src, .. } => contains(src, pred),
            PhysicalRelExpr::CrossJoin { left, right, .. }
            | PhysicalRelExpr::NestedLoopJoin { left, right, .. }
            | PhysicalRelExpr::HashJoin { left, right, .. }
            | PhysicalRelExpr::SortMergeJoin { left, right, .. } => {
                contains(left, pred) || contains(right, pred)
            }
            PhysicalRelExpr::Scan { .. } | PhysicalRelExpr::IndexScan { .. } => false,
        }
    }

    fn is_hash_join(p: &PhysicalRelExpr) -> bool {
        matches!(p, PhysicalRelExpr::HashJoin { .. })
    }
    fn is_nested_loop_join(p: &PhysicalRelExpr) -> bool {
        matches!(p, PhysicalRelExpr::NestedLoopJoin { .. })
    }
    fn is_index_scan(p: &PhysicalRelExpr) -> bool {
        matches!(p, PhysicalRelExpr::IndexScan { .. })
    }

    fn optimize_sql(sql: &str, catalog: &Arc<Catalog>, managers: &'static queryexe::Managers) -> PhysicalRelExpr {
        let ast = testutil::parse_sql(sql);
        let enabled_rules = Arc::new(Rules::default());
        let col_id_gen = Arc::new(ColIdGenerator::new());
        let mut translator = Translator::new(catalog, &enabled_rules, &col_id_gen);
        let query = translator.process_query(&ast).unwrap();

        let cost_model = CardinalityCostModel::new(managers.stats);
        let optimizer = CascadesOptimizer::new(cost_model, managers, catalog.clone());
        optimizer.optimize(&query, None)
    }

    #[test]
    fn equi_join_picks_hash_join_over_nested_loop() {
        testutil::init();
        let catalog = testutil::get_test_catalog();
        let config: &'static ServerConfig = Box::leak(Box::new(ServerConfig::temporary()));
        let managers = testutil::get_managers(config, Some(catalog.clone()), (30, 30, 1));

        let plan = optimize_sql("select * from t1 join t2 on t1.a = t2.c", &catalog, managers);
        assert!(contains(&plan, is_hash_join), "expected a HashJoin in:\n{}", plan.pretty_string());
        assert!(
            !contains(&plan, is_nested_loop_join),
            "did not expect a NestedLoopJoin in:\n{}",
            plan.pretty_string()
        );
    }

    #[test]
    fn non_equi_join_has_no_alternative_to_nested_loop() {
        testutil::init();
        let catalog = testutil::get_test_catalog();
        let config: &'static ServerConfig = Box::leak(Box::new(ServerConfig::temporary()));
        let managers = testutil::get_managers(config, Some(catalog.clone()), (5, 5, 1));

        let plan = optimize_sql("select * from t1 join t2 on t1.a < t2.c", &catalog, managers);
        assert!(
            contains(&plan, is_nested_loop_join),
            "expected a NestedLoopJoin (no hash join alternative for a non-equality predicate) in:\n{}",
            plan.pretty_string()
        );
    }

    /// Build a single-column table with a deliberately skewed value
    /// distribution and a secondary index on that column, so a highly
    /// selective vs. a barely selective equality predicate can be compared.
    fn setup_skewed_table(majority_count: usize, minority_value: i64) -> (Arc<Catalog>, &'static queryexe::Managers) {
        let catalog = Catalog::new();
        let schema = TableSchema::from_vecs(vec!["v"], vec![DataType::BigInt]);
        let table_id = catalog.get_table_id("skewed");
        catalog.add_table(TableInfo::new(table_id, "skewed".to_string(), schema.clone()));

        let sm = StorageManager::new_test_sm();
        sm.create_container(table_id, Some("skewed".to_string()), StateType::BaseTable, None)
            .unwrap();
        let stats = ReservoirStatManager::new_test_stat_manager();
        stats.register_table(table_id, schema).unwrap();

        let tid = TransactionId::new();
        let mut rows = Vec::with_capacity(majority_count + 1);
        for _ in 0..majority_count {
            rows.push(Tuple::new(vec![Field::BigInt(1)]));
        }
        rows.push(Tuple::new(vec![Field::BigInt(minority_value)]));
        let ids = sm.insert_values(table_id, rows.iter().map(|t| t.to_bytes()).collect(), tid);
        for (row, id) in rows.iter().zip(ids.iter()) {
            stats.new_record(row, *id).unwrap();
        }

        catalog.add_index(IndexInfo::new(
            catalog.get_table_id("idx_skewed_v"),
            "idx_skewed_v".to_string(),
            table_id,
            vec![0],
            false,
        ));

        let sm: &'static StorageManager = Box::leak(Box::new(sm));
        let stats: &'static ReservoirStatManager = Box::leak(Box::new(stats));
        let config: &'static ServerConfig = Box::leak(Box::new(ServerConfig::temporary()));
        let tm = testutil::get_tm(config);
        let im: &'static IndexManager = testutil::get_im(config, sm, tm);
        let strm: &'static StringManager = Box::leak(Box::new(StringManager::new(config, 1024 * 100, 0)));
        let managers: &'static queryexe::Managers =
            Box::leak(Box::new(queryexe::Managers::new(config, sm, tm, im, stats, strm)));

        (catalog, managers)
    }

    #[test]
    fn highly_selective_predicate_uses_index_scan() {
        testutil::init();
        let (catalog, managers) = setup_skewed_table(999, 999);
        let plan = optimize_sql("select * from skewed where v = 999", &catalog, managers);
        assert!(
            contains(&plan, is_index_scan),
            "expected an IndexScan for a 1-in-1000 selective predicate in:\n{}",
            plan.pretty_string()
        );
    }

    #[test]
    fn optimized_join_executes_correctly() {
        // Shape (equi_join_picks_hash_join_over_nested_loop) is one thing;
        // make sure the HashJoin CascadesOptimizer picks actually runs and
        // returns the right rows through the normal opiterator pipeline.
        testutil::init();
        let catalog = testutil::get_test_catalog();
        let config: &'static ServerConfig = Box::leak(Box::new(ServerConfig::temporary()));
        let managers = testutil::get_managers(config, Some(catalog.clone()), (1, 1, 1));

        let ast = testutil::parse_sql("select t1.a, t2.c from t1 join t2 on t1.a = t2.c");
        let enabled_rules = Arc::new(Rules::default());
        let col_id_gen = Arc::new(ColIdGenerator::new());
        let mut translator = Translator::new(&catalog, &enabled_rules, &col_id_gen);
        let query = translator.process_query(&ast).unwrap();

        let cost_model = CardinalityCostModel::new(managers.stats);
        let optimizer = CascadesOptimizer::new(cost_model, managers, catalog.clone());
        let plan = optimizer.optimize(&query, None);
        assert!(contains(&plan, is_hash_join));

        let mut op = queryexe::query::planner::physical_plan_to_op_iterator(
            managers,
            &catalog,
            &plan,
            TransactionId::new(),
            0,
        )
        .unwrap();
        op.configure(false);
        op.open().unwrap();
        let rows = testutil::get_results(&mut op);
        // t1.a and t2.c both range 1..=5, so every row matches exactly once.
        assert_eq!(rows.len(), 5, "unexpected row count from optimized join: {:?}", rows);
    }

    #[test]
    fn low_selectivity_predicate_uses_full_scan() {
        testutil::init();
        let (catalog, managers) = setup_skewed_table(999, 999);
        let plan = optimize_sql("select * from skewed where v = 1", &catalog, managers);
        assert!(
            !contains(&plan, is_index_scan),
            "expected the cost model to reject the index for a 999-in-1000 predicate in:\n{}",
            plan.pretty_string()
        );
    }
}
