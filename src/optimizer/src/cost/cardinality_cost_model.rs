//! A real, statistics-driven cost model, replacing `DummyCostModel`'s
//! constant-zero cost with actual cardinality estimates from
//! `ReservoirStatManager` (which was already collecting real per-table,
//! per-attribute statistics — nothing previously consumed them for costing).
//!
//! The cost unit is "estimated rows of work": a `Scan` costs its table's row
//! count, a `Select` costs its child's cost plus its own estimated output
//! row count, a join costs its children's costs plus an estimate of the
//! join's own work (probe-proportional for `HashJoin`, product-proportional
//! for `NestedLoopJoin`). This is a standard first-approximation cost unit —
//! it doesn't model I/O vs CPU weighting or memory pressure, but it's enough
//! to make genuine, data-dependent choices instead of a fixed syntactic rule.

use common::ids::ContainerId;
use common::physical_expr::physical_rel_expr::PhysicalRelExpr;
use common::query::expr::Expression;
use common::query::origin_expr::OriginExpression;
use common::traits::stat_manager_trait::StatManagerTrait;
use queryexe::query::translate_and_validate::Environment;
use queryexe::stats::reservoir_stat_manager::ReservoirStatManager;

use super::dummy_cost_model::DummyCost;

/// What `Memo`/`CascadesOptimizer` need from a cost model: given a candidate
/// physical implementation of a node and the already-computed costs of its
/// children, produce this node's own cost. Deliberately not the pre-existing
/// `CostModel` trait in this module (that one is built around a
/// `GroupId`/`MemoNodeRefWrapper` memo API that was never completed — see
/// its "TODO milestone qo" marker — and doesn't give a cost function access
/// to the things it actually needs: the candidate node itself and its
/// children's costs).
pub trait PlanCostEstimator {
    type Cost: super::Cost;

    /// `env` resolves a physical plan's (possibly renamed) column ids back
    /// to the base table column they originated from, which is what the
    /// statistics manager's `estimate_count_and_sel`/`estimate_join_count_and_sel`
    /// need (see `StatManagerTrait`'s doc comments).
    fn cost_of(&self, node: &PhysicalRelExpr, child_costs: &[Self::Cost], env: &Environment) -> Self::Cost;

    /// Estimated number of rows in container `cid` matching `predicates`.
    /// Exposed separately from `cost_of` because `Memo` sometimes knows
    /// exactly which predicates a candidate corresponds to even when the
    /// candidate's own `PhysicalRelExpr` shape doesn't carry that
    /// information — `IndexScan`'s `key_values` are bare literals with no
    /// column reference, so costing it accurately needs this called
    /// directly with the equality predicates reconstructed from the
    /// catalog's `IndexInfo` (see `memo.rs`).
    fn estimate_matching_rows(
        &self,
        cid: ContainerId,
        predicates: &[Expression<PhysicalRelExpr>],
        env: &Environment,
    ) -> f64;

    /// Cost of probing an index on `cid` for rows matching `predicates`
    /// (the equality conditions reconstructed from the catalog's
    /// `IndexInfo` — see `memo.rs`), used to price an `IndexScan` candidate
    /// with the same selectivity awareness `estimate_matching_rows` gives
    /// `Select`, instead of a size-only guess.
    fn cost_of_index_probe(
        &self,
        cid: ContainerId,
        predicates: &[Expression<PhysicalRelExpr>],
        env: &Environment,
    ) -> Self::Cost;

    /// Cost of a full scan of `cid` followed by filtering on `predicates`
    /// (already raw-indexed, like `cost_of_index_probe`'s) -- the other side
    /// of the `IndexScan`-vs-scan comparison. Kept separate from `cost_of`'s
    /// generic `Select` handling for the same reason `cost_of_index_probe`
    /// is: `Memo` builds this alternative's actual predicate using *temp*
    /// column ids (what `Scan`+`Select` needs to execute), which aren't
    /// resolvable through `env` the way ordinary query predicates are.
    fn cost_of_full_scan_with_filter(
        &self,
        cid: ContainerId,
        raw_predicates: &[Expression<PhysicalRelExpr>],
    ) -> Self::Cost;
}

#[derive(Clone)]
pub struct CardinalityCostModel {
    stats: &'static ReservoirStatManager,
}

impl CardinalityCostModel {
    pub fn new(stats: &'static ReservoirStatManager) -> Self {
        Self { stats }
    }

    /// Rewrite every `ColRef` in `expr` from the query's renamed column id
    /// space down to the base table's raw (0-based) attribute index, via the
    /// translator's origin map. Falls back to leaving a `ColRef` unresolved
    /// (rather than erroring) if the origin map can't place it — the
    /// resulting predicate still evaluates the same way structurally, it
    /// just won't match statistics keyed by raw index, which only degrades
    /// the cost *estimate*, not correctness of the plan itself.
    fn resolve_to_raw(expr: &Expression<PhysicalRelExpr>, env: &Environment) -> Expression<PhysicalRelExpr> {
        match expr {
            Expression::ColRef { id } => {
                match env.get_origin(&OriginExpression::DerivedColRef { col_id: *id }) {
                    OriginExpression::BaseCidAndIndex { index, .. } => Expression::ColRef { id: index },
                    _ => expr.clone(),
                }
            }
            Expression::Field { .. } => expr.clone(),
            Expression::Binary { op, left, right } => Expression::Binary {
                op: *op,
                left: Box::new(Self::resolve_to_raw(left, env)),
                right: Box::new(Self::resolve_to_raw(right, env)),
            },
            Expression::Case { expr, whens, else_expr } => Expression::Case {
                expr: Box::new(Self::resolve_to_raw(expr, env)),
                whens: whens
                    .iter()
                    .map(|(w, t)| (Self::resolve_to_raw(w, env), Self::resolve_to_raw(t, env)))
                    .collect(),
                else_expr: Box::new(Self::resolve_to_raw(else_expr, env)),
            },
            // Subqueries aren't costed independently here; leave as-is.
            Expression::Subquery { .. } => expr.clone(),
        }
    }

    /// The single base table `node` scans, if it scans exactly one (a Scan,
    /// IndexScan, or any chain of single-input operators sitting on one of
    /// those). `None` for joins or multi-table shapes.
    fn single_table_cid(node: &PhysicalRelExpr) -> Option<ContainerId> {
        let mut cids = Vec::new();
        node.get_tables_involved(&mut cids);
        cids.dedup();
        match cids.as_slice() {
            [cid] => Some(*cid),
            _ => None,
        }
    }

    fn estimate_rows(&self, cid: ContainerId) -> f64 {
        self.stats.get_container_record_count(cid).unwrap_or(0) as f64
    }
}

/// A point lookup via an index touches pages essentially at random, while a
/// sequential scan reads them in physical order; a real buffer pool amortizes
/// sequential reads far better. This mirrors the classic
/// `random_page_cost`/`seq_page_cost` ratio real optimizers (e.g. Postgres)
/// use — real, if approximate, and it's what actually makes the
/// IndexScan-vs-Scan choice selectivity-dependent instead of the index
/// always numerically winning regardless of how many rows it matches.
const RANDOM_ACCESS_PENALTY: f64 = 4.0;

impl PlanCostEstimator for CardinalityCostModel {
    type Cost = DummyCost;

    fn estimate_matching_rows(
        &self,
        cid: ContainerId,
        predicates: &[Expression<PhysicalRelExpr>],
        env: &Environment,
    ) -> f64 {
        let resolved: Vec<Expression<PhysicalRelExpr>> =
            predicates.iter().map(|p| Self::resolve_to_raw(p, env)).collect();
        self.stats
            .estimate_count_and_sel(cid, &resolved)
            .map(|(count, _sel)| count as f64)
            .unwrap_or_else(|_| self.estimate_rows(cid))
    }

    fn cost_of_index_probe(
        &self,
        cid: ContainerId,
        predicates: &[Expression<PhysicalRelExpr>],
        _env: &Environment,
    ) -> DummyCost {
        // Unlike `estimate_matching_rows`, `predicates` here are already
        // raw-indexed (`Memo` builds them straight from `IndexInfo.columns`,
        // not from query-level column ids) -- resolving them through `env`
        // would either be a no-op or, worse, misresolve/panic on an id that
        // was never renamed. Go straight to the stats manager.
        let n = self.estimate_rows(cid).max(2.0);
        let matches = self
            .stats
            .estimate_count_and_sel(cid, predicates)
            .map(|(count, _sel)| count as f64)
            .unwrap_or_else(|_| self.estimate_rows(cid));
        DummyCost::new(n.log2() + matches * RANDOM_ACCESS_PENALTY)
    }

    fn cost_of_full_scan_with_filter(
        &self,
        cid: ContainerId,
        raw_predicates: &[Expression<PhysicalRelExpr>],
    ) -> DummyCost {
        let n = self.estimate_rows(cid);
        let matches = self
            .stats
            .estimate_count_and_sel(cid, raw_predicates)
            .map(|(count, _sel)| count as f64)
            .unwrap_or(n);
        DummyCost::new(n + matches)
    }

    fn cost_of(&self, node: &PhysicalRelExpr, child_costs: &[Self::Cost], env: &Environment) -> DummyCost {
        let children_total: f64 = child_costs.iter().fold(0.0, |acc, c| acc + c.value());

        let own = match node {
            PhysicalRelExpr::Scan { cid, .. } => self.estimate_rows(*cid),

            PhysicalRelExpr::IndexScan { cid, .. } => {
                // `PhysicalRelExpr::IndexScan.key_values` are bare literals
                // with no column reference, so we can't recover which raw
                // column(s) they constrain here (only `IndexInfo`, in the
                // catalog, has that) -- `Memo` calls `estimate_matching_rows`
                // directly with the properly-resolved predicates instead and
                // overrides this candidate's cost (see `memo.rs`). This
                // fallback (log2(N) tree depth + a handful of rows) is only
                // reached when that sharper path isn't available.
                let n = self.estimate_rows(*cid).max(2.0);
                n.log2() + RANDOM_ACCESS_PENALTY
            }

            PhysicalRelExpr::Select { src, predicates, .. } => {
                match Self::single_table_cid(src) {
                    Some(cid) => self.estimate_matching_rows(cid, predicates, env),
                    // Filtering on top of a multi-table shape (e.g. after a
                    // join): no single-table stats apply, so fall back to
                    // the (already-computed) input size unchanged.
                    None => children_total,
                }
            }

            PhysicalRelExpr::NestedLoopJoin { left, right, .. } => {
                let (left_cid, right_cid) = (Self::single_table_cid(left), Self::single_table_cid(right));
                let (l, r) = (
                    left_cid.map(|c| self.estimate_rows(c)).unwrap_or(children_total),
                    right_cid.map(|c| self.estimate_rows(c)).unwrap_or(children_total),
                );
                // O(n*m): every row of the outer probes every row of the inner.
                l * r.max(1.0)
            }

            PhysicalRelExpr::HashJoin { left, right, .. } => {
                // Deliberately *not* `estimate_join_count_and_sel`: that
                // estimator is a nested loop over the two sides' reservoir
                // samples (O(sample_size^2)), which is fine for an
                // occasional selectivity query but far too expensive to
                // call on every optimization of every join query -- for
                // small/test-scale tables the reservoir is close to the
                // full table, making the "estimate" cost about as much
                // work as the join itself. A hash join's own cost is
                // genuinely dominated by build+probe, i.e. the *input*
                // sizes, not its output cardinality, so this linear proxy
                // doesn't lose the property that actually matters here:
                // it's always cheaper than `NestedLoopJoin`'s O(n*m).
                let (left_cid, right_cid) = (Self::single_table_cid(left), Self::single_table_cid(right));
                let (l, r) = (
                    left_cid.map(|c| self.estimate_rows(c)).unwrap_or(children_total),
                    right_cid.map(|c| self.estimate_rows(c)).unwrap_or(children_total),
                );
                l + r
            }

            PhysicalRelExpr::SortMergeJoin { left, right, .. } => {
                let (left_cid, right_cid) = (Self::single_table_cid(left), Self::single_table_cid(right));
                let (l, r) = (
                    left_cid.map(|c| self.estimate_rows(c)).unwrap_or(children_total),
                    right_cid.map(|c| self.estimate_rows(c)).unwrap_or(children_total),
                );
                // Sort dominates: n*log(n) + m*log(m).
                l * (l.max(2.0).log2()) + r * (r.max(2.0).log2())
            }

            // No competing physical alternative exists for these in this
            // codebase, so their cost never influences a decision -- just
            // pass the input volume through so ancestor nodes still see a
            // sensible running total.
            PhysicalRelExpr::CrossJoin { .. }
            | PhysicalRelExpr::Project { .. }
            | PhysicalRelExpr::Sort { .. }
            | PhysicalRelExpr::HashAggregate { .. }
            | PhysicalRelExpr::Map { .. }
            | PhysicalRelExpr::FlatMap { .. }
            | PhysicalRelExpr::Rename { .. } => children_total,

            PhysicalRelExpr::Delete { .. } | PhysicalRelExpr::Update { .. } => children_total,
        };

        DummyCost::new(children_total + own)
    }
}

