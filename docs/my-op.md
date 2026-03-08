# Write up

Sasha Timokhin

## Query Life-Cycle Question

`SELECT * FROM table WHERE a > 10;`

### 1. Client to Server

The client sends the SQL string over a TCP socket. The server receives it in `server.rs`, dispatches it to a worker thread, which calls:

```
handler::handle_database_command() -> handler::run_database_command()
```

### 2. Parsing

```
conductor::run_sql_from_string() -> SQLParser::parse_sql()
```

The SQL string is parsed into an abstract syntax tree (AST) using `sqlparser`. For a `SELECT` statement, `run_sql` matches on `Statement::Query`.

### 3. Logical Plan

```
conductor::run_sql() -> Translator::from_sql()
```

`Translator::from_sql` walks the AST and builds a logical plan. For `SELECT * FROM table WHERE a > 10`, this produces a logical `Filter` node on top of a logical `Scan` node, with the predicate `a > 10` represented as a bytecode expression.

### 4. Physical Plan

```
conductor::run_sql() -> self.optimizer.optimize(&lp, ...)
```

The MockOptimizer converts the logical plan into a physical plan via `LogicalRelExpr::to_physical_plan`. For this query, the optimizer produces a physical `SeqScan` with an inlined filter expression (the `WHERE a > 10` predicate is pushed into the scan).

### 5. OpIterator Tree

```
conductor::run_physical_plan() -> physical_plan_to_op_iterator() -> executor.configure_query(op_iterator)
```

The physical plan is converted into a tree of `OpIterator` objects. Since the only operators are a filtered scan with a wildcard projection, the tree is just a single `SeqScan` node that holds the `a > 10` predicate as a `ByteCodeExpr`.

### 6. Execution

```
executor::execute() -> SeqScan::open() -> sm.get_iterator() -> SeqScan::next() -> HeapFileIter::next()
```

`executor::execute()` calls `configure()`, then `open()`, then loops on `next()`. `SeqScan::open()` calls `sm.get_iterator()` to create a `HeapFileIter`. 

On the first call to `SeqScan::next()`, `HeapFileIter::next()` calls `HeapFileIter::initialize()`, which calls `HeapFile::get_page_for_read()` — this is when the storage manager reads the first page from the HeapFile. For each tuple, `SeqScan::next()` calls `Tuple::from_bytes()` to deserialize it, evaluates the `a > 10` filter predicate, and returns the tuple if it passes.

### When Does the Storage Manager Read a Page?

The storage manager reads a page at the **first call to `SeqScan::next()`**, inside `HeapFileIter::initialize()` -> `HeapFile::get_page_for_read()`. If the page is not already in the buffer pool, it is fetched from disk at this point. Each subsequent page is read when the iterator exhausts all slots on the current page and advances to the next one.

## Design

### NestedLoopJoin

The implementation follows the same outer-loop / inner-loop pattern as `CrossJoin`, but adds a predicate check before yielding each tuple pair. `current_tuple` holds the current left tuple; the right child is fully rewound for each new left tuple. `left_expr` and `right_expr` are evaluated against each pair, and `compare_fields(op, &left_val, &right_val)` determines whether the pair is emitted. Tuples are merged left-first via `Tuple::merge`.

### HashEqJoin

The build phase happens entirely inside `open()`: the left child is consumed and each tuple is hashed by `left_expr.eval()` into a `HashMap<Field, Vec<Tuple>>`. The right child is then opened and its first tuple is fetched into `current_tuple`. In `next()`, each right tuple is probed against the hash map using `right_expr.eval()`. `current_idx` tracks position within a matching bucket, allowing multiple left tuples to be matched to a single right tuple without re-scanning the map. Output tuples are left-merged-with-right.

Left child is never rewound (it is fully consumed once in `open()`), while the right child may be rewound (via `rewind()`) if the operator itself needs to be rewound.

### Aggregate

`open()` is a blocking phase: it drains the child entirely, calling `merge_tuple_into_group()` for each tuple. Groups are keyed by `Vec<Field>` (the evaluated `groupby_expr` values). Each group stores a `(count, Vec<Field>)` pair in the `acc` HashMap, where the Vec holds running aggregate values indexed to match `ops`.

For `COUNT`, new groups initialize with `Field::BigInt(1)` so that the existing `merge_fields` (which adds 1) is only called for subsequent tuples. All other ops initialize with the first field value directly. `AVG` accumulates a running sum; the final division by count happens during the post-processing step in `open()`, converting to `f_decimal`.

After all groups are built, entries are sorted by group key (so output is deterministic) and final `acc_iter: Vec<Tuple>` is constructed. `next()` and `rewind()` are then simple index operations over this pre-built vector, since Aggregate is a fully blocking operator.

## Time Estimate / Reflection

Approximately 30 hours total.

## Incomplete

N/A

## References

- `src/queryexe/src/opiterator/cross_join.rs` — reference implementation for the join outer-loop / inner-rewind pattern
- `src/queryexe/src/opiterator/filter.rs` — reference for bytecode predicate evaluation via `ByteCodeExpr::eval()`
- `src/common/src/datatypes.rs` — `compare_fields()` function and `f_decimal()` helper
- `src/queryexe/src/opiterator/seqscan.rs` — reference for how the storage manager iterator is used
- `src/server/src/conductor.rs` and `src/server/src/handler.rs` — traced the query lifecycle from SQL string to executor
