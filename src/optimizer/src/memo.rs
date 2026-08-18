//! A deliberately simplified Cascades-style memo: every distinct position in
//! the original logical plan tree gets its own group (no structural
//! deduplication of syntactically-identical subtrees — real Cascades-style
//! subplan sharing is not implemented), and for two node kinds where a real
//! implementation choice exists — join algorithm, and (when built with
//! catalog access) index-scan vs. full-scan-plus-filter — more than one
//! physical candidate is generated per group and costed. Everything else has
//! exactly one physical form, matching `LogicalRelExpr::to_physical_plan()`.
//!
//! This intentionally does not attempt full plan-space search (join
//! reordering, rule-driven exploration): it's a bottom-up, System-R-style
//! "pick the best physical form for each subplan, once" optimizer, which is
//! enough to make genuine, data-dependent decisions instead of the fixed
//! syntactic rules `to_physical_plan()` uses, without the scope of a full
//! Cascades search engine.

use std::cell::RefCell;
use std::collections::HashMap;

use common::catalog::CatalogRef;
use common::ids::{ColumnId, ContainerId, GroupId};
use common::logical_expr::prelude::{JoinType, LogicalRelExpr};
use common::physical_expr::physical_rel_expr::PhysicalRelExpr;
use common::query::expr::Expression;
use common::{AggOp, BinaryOp};

use crate::cost::cardinality_cost_model::PlanCostEstimator;
use crate::cost::Cost;

/// One node of the original logical plan, with children replaced by
/// `GroupId`s pointing at their own memo groups.
#[derive(Clone)]
enum LogicalNode {
    Scan {
        cid: ContainerId,
        table_name: String,
        column_names: Vec<ColumnId>,
    },
    IndexScan {
        cid: ContainerId,
        table_name: String,
        column_names: Vec<ColumnId>,
        index_id: ContainerId,
        key_values: Vec<Expression<LogicalRelExpr>>,
        /// The equivalent `Scan` + equality-`Select` alternative, reconstructed
        /// from the catalog's `IndexInfo` when the memo is built with catalog
        /// access (see `add_group_from_plan_with_catalog`). `None` when built
        /// without it (`add_group_from_plan`), in which case `IndexScan` is
        /// the only candidate for this group — whatever the translator's
        /// rewrite rule already decided stands.
        scan_select_alt: Option<ScanSelectAlt>,
    },
    Select {
        src: GroupId,
        predicates: Vec<Expression<LogicalRelExpr>>,
    },
    Join {
        join_type: JoinType,
        left: GroupId,
        right: GroupId,
        predicates: Vec<Expression<LogicalRelExpr>>,
    },
    Project {
        src: GroupId,
        cols: Vec<ColumnId>,
    },
    OrderBy {
        src: GroupId,
        cols: Vec<(ColumnId, bool, bool)>,
    },
    Aggregate {
        src: GroupId,
        group_by: Vec<ColumnId>,
        aggrs: Vec<(ColumnId, (ColumnId, AggOp))>,
    },
    Map {
        input: GroupId,
        exprs: Vec<(ColumnId, Expression<LogicalRelExpr>)>,
    },
    FlatMap {
        input: GroupId,
        func: GroupId,
    },
    Rename {
        src: GroupId,
        src_to_dest: HashMap<ColumnId, ColumnId>,
    },
    /// `Delete`/`Update` (always plan roots — see their doc comments on
    /// `PhysicalRelExpr`) and anything else with no competing physical
    /// alternative worth memo-izing: delegate straight to the existing
    /// structural translation.
    Opaque(LogicalRelExpr),
}

/// Two representations of the same equality predicates, kept separate
/// because they live in different column-id spaces:
/// - `plan_predicates` use the *temp* column ids `Scan`'s own `column_names`
///   use (`get_temp_col_id(cid, raw_idx)`), which is what a `Select` sitting
///   directly on a plain `Scan` needs to actually execute (matches how
///   `planner.rs` resolves a `Scan`'s output columns).
/// - `raw_predicates` use the plain 0-based per-table attribute index
///   `StatManagerTrait::estimate_count_and_sel` expects directly, with no
///   further resolution — using `Environment::get_origin` on these would
///   panic, since it only knows about *renamed* (post-`.rename()`) column
///   ids, not temp ids or raw indices.
#[derive(Clone)]
struct ScanSelectAlt {
    plan_columns: Vec<ColumnId>,
    plan_predicates: Vec<Expression<LogicalRelExpr>>,
    raw_predicates: Vec<Expression<LogicalRelExpr>>,
}

struct MemoGroup<C: Cost> {
    node: LogicalNode,
    winner: RefCell<Option<(PhysicalRelExpr, C)>>,
}

pub struct Memo<C: Cost> {
    groups: RefCell<Vec<MemoGroup<C>>>,
}

impl<C: Cost> Default for Memo<C> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C: Cost> Memo<C> {
    pub fn new() -> Self {
        Self {
            groups: RefCell::new(Vec::new()),
        }
    }

    fn new_group(&self, node: LogicalNode) -> GroupId {
        let mut groups = self.groups.borrow_mut();
        let id = groups.len();
        groups.push(MemoGroup {
            node,
            winner: RefCell::new(None),
        });
        id
    }

    /// Build the memo's group table from `plan`, without catalog access —
    /// `IndexScan` groups get only the one candidate the translator already
    /// picked.
    pub fn add_group_from_plan(&self, plan: &LogicalRelExpr) -> GroupId {
        self.add_group_inner(plan, None)
    }

    /// Same, but with catalog access so an `IndexScan` group can also carry
    /// the equivalent `Scan`+`Select` as a second candidate, letting a real
    /// cost model decide between them instead of the translator's rewrite
    /// rule being final.
    pub fn add_group_from_plan_with_catalog(&self, plan: &LogicalRelExpr, catalog: &CatalogRef) -> GroupId {
        self.add_group_inner(plan, Some(catalog))
    }

    fn add_group_inner(&self, plan: &LogicalRelExpr, catalog: Option<&CatalogRef>) -> GroupId {
        match plan {
            LogicalRelExpr::Scan {
                cid,
                table_name,
                column_names,
            } => self.new_group(LogicalNode::Scan {
                cid: *cid,
                table_name: table_name.clone(),
                column_names: column_names.clone(),
            }),
            LogicalRelExpr::IndexScan {
                cid,
                table_name,
                column_names,
                index_id,
                key_values,
            } => {
                let scan_select_alt = catalog.and_then(|catalog| {
                    let info = catalog
                        .get_indexes_for_table(*cid)
                        .into_iter()
                        .find(|i| i.index_id == *index_id)?;
                    let mut plan_predicates = Vec::with_capacity(info.columns.len());
                    let mut raw_predicates = Vec::with_capacity(info.columns.len());
                    for (raw_idx, kv) in info.columns.iter().zip(key_values.iter()) {
                        let plan_col_id = *column_names.get(*raw_idx)?;
                        plan_predicates.push(Expression::Binary {
                            op: BinaryOp::Eq,
                            left: Box::new(Expression::ColRef { id: plan_col_id }),
                            right: Box::new(kv.clone()),
                        });
                        raw_predicates.push(Expression::Binary {
                            op: BinaryOp::Eq,
                            left: Box::new(Expression::ColRef { id: *raw_idx }),
                            right: Box::new(kv.clone()),
                        });
                    }
                    Some(ScanSelectAlt {
                        plan_columns: column_names.clone(),
                        plan_predicates,
                        raw_predicates,
                    })
                });
                self.new_group(LogicalNode::IndexScan {
                    cid: *cid,
                    table_name: table_name.clone(),
                    column_names: column_names.clone(),
                    index_id: *index_id,
                    key_values: key_values.clone(),
                    scan_select_alt,
                })
            }
            LogicalRelExpr::Select { src, predicates } => {
                let src = self.add_group_inner(src, catalog);
                self.new_group(LogicalNode::Select {
                    src,
                    predicates: predicates.clone(),
                })
            }
            LogicalRelExpr::Join {
                join_type,
                left,
                right,
                predicates,
            } => {
                let left = self.add_group_inner(left, catalog);
                let right = self.add_group_inner(right, catalog);
                self.new_group(LogicalNode::Join {
                    join_type: *join_type,
                    left,
                    right,
                    predicates: predicates.clone(),
                })
            }
            LogicalRelExpr::Project { src, cols } => {
                let src = self.add_group_inner(src, catalog);
                self.new_group(LogicalNode::Project {
                    src,
                    cols: cols.clone(),
                })
            }
            LogicalRelExpr::OrderBy { src, cols } => {
                let src = self.add_group_inner(src, catalog);
                self.new_group(LogicalNode::OrderBy {
                    src,
                    cols: cols.clone(),
                })
            }
            LogicalRelExpr::Aggregate {
                src,
                group_by,
                aggrs,
            } => {
                let src = self.add_group_inner(src, catalog);
                self.new_group(LogicalNode::Aggregate {
                    src,
                    group_by: group_by.clone(),
                    aggrs: aggrs.clone(),
                })
            }
            LogicalRelExpr::Map { input, exprs } => {
                let input = self.add_group_inner(input, catalog);
                self.new_group(LogicalNode::Map {
                    input,
                    exprs: exprs.clone(),
                })
            }
            LogicalRelExpr::FlatMap { input, func } => {
                let input = self.add_group_inner(input, catalog);
                let func = self.add_group_inner(func, catalog);
                self.new_group(LogicalNode::FlatMap { input, func })
            }
            LogicalRelExpr::Rename { src, src_to_dest } => {
                let src = self.add_group_inner(src, catalog);
                self.new_group(LogicalNode::Rename {
                    src,
                    src_to_dest: src_to_dest.clone(),
                })
            }
            LogicalRelExpr::Delete { .. } | LogicalRelExpr::Update { .. } => {
                self.new_group(LogicalNode::Opaque(plan.clone()))
            }
        }
    }

    /// Rebuild the owned `LogicalRelExpr` a group represents (inverse of
    /// `add_group_inner`). Only used by the no-cost-model path (`pretty_string`,
    /// `logical_to_physical`), which doesn't need per-group alternatives.
    fn reconstruct_logical(&self, gid: GroupId) -> LogicalRelExpr {
        let node = self.groups.borrow()[gid].node.clone();
        match node {
            LogicalNode::Scan {
                cid,
                table_name,
                column_names,
            } => LogicalRelExpr::Scan {
                cid,
                table_name,
                column_names,
            },
            LogicalNode::IndexScan {
                cid,
                table_name,
                column_names,
                index_id,
                key_values,
                ..
            } => LogicalRelExpr::IndexScan {
                cid,
                table_name,
                column_names,
                index_id,
                key_values,
            },
            LogicalNode::Select { src, predicates } => LogicalRelExpr::Select {
                src: Box::new(self.reconstruct_logical(src)),
                predicates,
            },
            LogicalNode::Join {
                join_type,
                left,
                right,
                predicates,
            } => LogicalRelExpr::Join {
                join_type,
                left: Box::new(self.reconstruct_logical(left)),
                right: Box::new(self.reconstruct_logical(right)),
                predicates,
            },
            LogicalNode::Project { src, cols } => LogicalRelExpr::Project {
                src: Box::new(self.reconstruct_logical(src)),
                cols,
            },
            LogicalNode::OrderBy { src, cols } => LogicalRelExpr::OrderBy {
                src: Box::new(self.reconstruct_logical(src)),
                cols,
            },
            LogicalNode::Aggregate {
                src,
                group_by,
                aggrs,
            } => LogicalRelExpr::Aggregate {
                src: Box::new(self.reconstruct_logical(src)),
                group_by,
                aggrs,
            },
            LogicalNode::Map { input, exprs } => LogicalRelExpr::Map {
                input: Box::new(self.reconstruct_logical(input)),
                exprs,
            },
            LogicalNode::FlatMap { input, func } => LogicalRelExpr::FlatMap {
                input: Box::new(self.reconstruct_logical(input)),
                func: Box::new(self.reconstruct_logical(func)),
            },
            LogicalNode::Rename { src, src_to_dest } => LogicalRelExpr::Rename {
                src: Box::new(self.reconstruct_logical(src)),
                src_to_dest,
            },
            LogicalNode::Opaque(plan) => plan,
        }
    }

    pub fn pretty_string(&self, gid: GroupId) -> String {
        self.reconstruct_logical(gid).pretty_string()
    }

    /// Resolve every group to a physical plan using only structural rules
    /// (no cost model): the same join-algorithm rule `to_physical_plan()`
    /// applies, and — since this path never has catalog access — `IndexScan`
    /// groups stand as the translator already decided. Used where a
    /// definite plan is needed without genuine cost comparison (see
    /// `Memo`-level docs and this module's tests).
    pub fn logical_to_physical(&self, gid: GroupId) {
        let phys = self.reconstruct_logical(gid).to_physical_plan();
        *self.groups.borrow()[gid].winner.borrow_mut() = Some((phys, C::default()));
    }

    pub fn get_best_group_binding(&self, gid: GroupId) -> PhysicalRelExpr {
        self.groups.borrow()[gid]
            .winner
            .borrow()
            .as_ref()
            .expect("group has not been resolved (call logical_to_physical or resolve_costed first)")
            .0
            .clone()
    }

    /// The real, cost-driven resolution: recursively resolve every child
    /// group first (memoized — each group is only priced once), generate
    /// every physical candidate for `gid`'s own node using those resolved
    /// children, cost each with `cost_model`, and keep the cheapest.
    pub fn resolve_costed<M: PlanCostEstimator<Cost = C>>(
        &self,
        gid: GroupId,
        cost_model: &M,
        env: &queryexe::query::translate_and_validate::Environment,
    ) -> (PhysicalRelExpr, C) {
        if let Some(winner) = self.groups.borrow()[gid].winner.borrow().clone() {
            return winner;
        }
        let node = self.groups.borrow()[gid].node.clone();

        // Each candidate carries an optional pre-computed cost, used only by
        // `IndexScan` (see below) where the generic `cost_model.cost_of`
        // can't reconstruct enough information from the `PhysicalRelExpr`
        // shape alone to price it accurately. Every other candidate is
        // priced uniformly via `cost_model.cost_of` in the scoring loop.
        let (child_costs, candidates): (Vec<C>, Vec<(PhysicalRelExpr, Option<C>)>) = match &node {
            LogicalNode::Scan {
                cid,
                table_name,
                column_names,
            } => (
                vec![],
                vec![(
                    PhysicalRelExpr::Scan {
                        cid: *cid,
                        table_name: table_name.clone(),
                        column_names: column_names.clone(),
                        tree_hash: None,
                    },
                    None,
                )],
            ),

            LogicalNode::IndexScan {
                cid,
                table_name,
                column_names,
                index_id,
                key_values,
                scan_select_alt,
            } => {
                let index_scan = PhysicalRelExpr::IndexScan {
                    cid: *cid,
                    table_name: table_name.clone(),
                    column_names: column_names.clone(),
                    index_id: *index_id,
                    key_values: key_values.iter().map(|e| e.to_physical_expression()).collect(),
                    tree_hash: None,
                };
                let cands = match scan_select_alt {
                    Some(alt) => {
                        let raw_physical_preds: Vec<Expression<PhysicalRelExpr>> =
                            alt.raw_predicates.iter().map(|e| e.to_physical_expression()).collect();
                        let index_cost = cost_model.cost_of_index_probe(*cid, &raw_physical_preds, env);
                        let scan_cost = cost_model.cost_of_full_scan_with_filter(*cid, &raw_physical_preds);

                        let scan = PhysicalRelExpr::Scan {
                            cid: *cid,
                            table_name: table_name.clone(),
                            column_names: alt.plan_columns.clone(),
                            tree_hash: None,
                        };
                        let select = PhysicalRelExpr::Select {
                            src: Box::new(scan),
                            predicates: alt
                                .plan_predicates
                                .iter()
                                .map(|e| e.to_physical_expression())
                                .collect(),
                            tree_hash: None,
                        };
                        vec![(index_scan, Some(index_cost)), (select, Some(scan_cost))]
                    }
                    // Built without catalog access (`add_group_from_plan`,
                    // not `..._with_catalog`): no reconstructed predicates
                    // to price accurately with, so fall back to
                    // `cost_of`'s size-only estimate for this candidate.
                    None => vec![(index_scan, None)],
                };
                (vec![], cands)
            }

            LogicalNode::Select { src, predicates } => {
                let (src_phys, src_cost) = self.resolve_costed(*src, cost_model, env);
                let cand = PhysicalRelExpr::Select {
                    src: Box::new(src_phys),
                    predicates: predicates.iter().map(|e| e.to_physical_expression()).collect(),
                    tree_hash: None,
                };
                (vec![src_cost], vec![(cand, None)])
            }

            LogicalNode::Join {
                join_type,
                left,
                right,
                predicates,
            } => {
                let (left_phys, left_cost) = self.resolve_costed(*left, cost_model, env);
                let (right_phys, right_cost) = self.resolve_costed(*right, cost_model, env);
                let physical_predicates: Vec<Expression<PhysicalRelExpr>> =
                    predicates.iter().map(|e| e.to_physical_expression()).collect();
                let combined = vec![Expression::combine_preds(physical_predicates.as_slice())];

                let mut cands = vec![(
                    PhysicalRelExpr::NestedLoopJoin {
                        join_type: *join_type,
                        left: Box::new(left_phys.clone()),
                        right: Box::new(right_phys.clone()),
                        predicates: combined.clone(),
                        tree_hash: None,
                    },
                    None,
                )];
                let is_single_eq = matches!(
                    combined.as_slice(),
                    [Expression::Binary { op: BinaryOp::Eq, .. }]
                );
                if *join_type == JoinType::Inner && is_single_eq {
                    cands.push((
                        PhysicalRelExpr::HashJoin {
                            join_type: *join_type,
                            left: Box::new(left_phys),
                            right: Box::new(right_phys),
                            predicates: combined,
                            tree_hash: None,
                        },
                        None,
                    ));
                }
                (vec![left_cost, right_cost], cands)
            }

            LogicalNode::Project { src, cols } => {
                let (src_phys, src_cost) = self.resolve_costed(*src, cost_model, env);
                let cand = PhysicalRelExpr::Project {
                    src: Box::new(src_phys),
                    cols: cols.clone(),
                    tree_hash: None,
                };
                (vec![src_cost], vec![(cand, None)])
            }

            LogicalNode::OrderBy { src, cols } => {
                let (src_phys, src_cost) = self.resolve_costed(*src, cost_model, env);
                let cand = PhysicalRelExpr::Sort {
                    src: Box::new(src_phys),
                    cols: cols.clone(),
                    tree_hash: None,
                };
                (vec![src_cost], vec![(cand, None)])
            }

            LogicalNode::Aggregate {
                src,
                group_by,
                aggrs,
            } => {
                let (src_phys, src_cost) = self.resolve_costed(*src, cost_model, env);
                let cand = PhysicalRelExpr::HashAggregate {
                    src: Box::new(src_phys),
                    group_by: group_by.clone(),
                    aggrs: aggrs.clone(),
                    tree_hash: None,
                };
                (vec![src_cost], vec![(cand, None)])
            }

            LogicalNode::Map { input, exprs } => {
                let (input_phys, input_cost) = self.resolve_costed(*input, cost_model, env);
                let cand = PhysicalRelExpr::Map {
                    input: Box::new(input_phys),
                    exprs: exprs
                        .iter()
                        .map(|(id, e)| (*id, e.to_physical_expression()))
                        .collect(),
                    tree_hash: None,
                };
                (vec![input_cost], vec![(cand, None)])
            }

            LogicalNode::FlatMap { input, func } => {
                let (input_phys, input_cost) = self.resolve_costed(*input, cost_model, env);
                let (func_phys, func_cost) = self.resolve_costed(*func, cost_model, env);
                let cand = PhysicalRelExpr::FlatMap {
                    input: Box::new(input_phys),
                    func: Box::new(func_phys),
                    tree_hash: None,
                };
                (vec![input_cost, func_cost], vec![(cand, None)])
            }

            LogicalNode::Rename { src, src_to_dest } => {
                let (src_phys, src_cost) = self.resolve_costed(*src, cost_model, env);
                let cand = PhysicalRelExpr::Rename {
                    src: Box::new(src_phys),
                    src_to_dest: src_to_dest.clone(),
                    tree_hash: None,
                };
                (vec![src_cost], vec![(cand, None)])
            }

            LogicalNode::Opaque(plan) => (vec![], vec![(plan.to_physical_plan(), None)]),
        };

        let mut best: Option<(PhysicalRelExpr, C)> = None;
        for (cand, precomputed) in candidates {
            let cost = precomputed.unwrap_or_else(|| cost_model.cost_of(&cand, &child_costs, env));
            let better = match &best {
                Some((_, best_cost)) => cost < *best_cost,
                None => true,
            };
            if better {
                best = Some((cand, cost));
            }
        }
        let winner = best.expect("resolve_costed: node produced no physical candidates");
        *self.groups.borrow()[gid].winner.borrow_mut() = Some(winner.clone());
        winner
    }
}
