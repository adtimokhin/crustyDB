use cli_crusty::Response;
use common::QueryResult;
use e2e_tests::test_db::TestDB;

fn run_ok(db: &mut TestDB, command: &str) -> Response {
    let res = db.send_command(command.as_bytes());
    assert_eq!(res.len(), 1, "command: {}", command);
    if matches!(
        res[0],
        Response::SystemErr(_) | Response::QuietErr | Response::QueryExecutionError(_)
    ) {
        panic!("command failed: {} -> {:?}", command, res[0]);
    }
    res.into_iter().next().unwrap()
}

fn select_count(db: &mut TestDB, sql: &str) -> usize {
    match run_ok(db, sql) {
        Response::QueryResult(QueryResult::Select { result, .. }) => result.len(),
        other => panic!("expected a Select result, got {:?}", other),
    }
}

#[test]
fn create_index_on_existing_rows_then_lookup() {
    let mut db = TestDB::new();
    run_ok(
        &mut db,
        "CREATE TABLE t (a INT, b INT, primary key (a));",
    );
    run_ok(&mut db, "\\i csv/large_data.csv t");

    // large_data.csv has 1000 rows with distinct `b` values (see prior GROUP BY check).
    run_ok(&mut db, "CREATE INDEX idx_t_b ON t (b);");

    let via_index = select_count(&mut db, "select * from t where b = 5;");
    assert_eq!(via_index, 1);
    assert_eq!(select_count(&mut db, "select * from t where b = 999999;"), 0);
}

#[test]
fn index_maintained_on_insert() {
    let mut db = TestDB::new();
    run_ok(&mut db, "CREATE TABLE t (a INT, b INT, primary key (a));");
    run_ok(&mut db, "CREATE INDEX idx_t_b ON t (b);");

    run_ok(&mut db, "INSERT INTO t VALUES (1, 100), (2, 200), (3, 300);");

    assert_eq!(select_count(&mut db, "select * from t where b = 200;"), 1);
    assert_eq!(select_count(&mut db, "select * from t where b = 400;"), 0);
}

#[test]
fn index_maintained_on_delete() {
    let mut db = TestDB::new();
    run_ok(&mut db, "CREATE TABLE t (a INT, b INT, primary key (a));");
    run_ok(&mut db, "INSERT INTO t VALUES (1, 100), (2, 200), (3, 300);");
    run_ok(&mut db, "CREATE INDEX idx_t_b ON t (b);");

    assert_eq!(select_count(&mut db, "select * from t where b = 200;"), 1);
    run_ok(&mut db, "DELETE FROM t WHERE a = 2;");
    assert_eq!(select_count(&mut db, "select * from t where b = 200;"), 0);
    // Untouched rows still findable.
    assert_eq!(select_count(&mut db, "select * from t where b = 100;"), 1);
    assert_eq!(select_count(&mut db, "select * from t where b = 300;"), 1);
}

#[test]
fn index_maintained_on_update() {
    let mut db = TestDB::new();
    run_ok(&mut db, "CREATE TABLE t (a INT, b INT, primary key (a));");
    run_ok(&mut db, "INSERT INTO t VALUES (1, 100), (2, 200), (3, 300);");
    run_ok(&mut db, "CREATE INDEX idx_t_b ON t (b);");

    run_ok(&mut db, "UPDATE t SET b = 999 WHERE a = 2;");

    // Old key no longer resolves, new key does.
    assert_eq!(select_count(&mut db, "select * from t where b = 200;"), 0);
    assert_eq!(select_count(&mut db, "select * from t where b = 999;"), 1);
}

#[test]
fn non_unique_secondary_index_returns_all_matches() {
    let mut db = TestDB::new();
    run_ok(&mut db, "CREATE TABLE t (a INT, b INT, primary key (a));");
    run_ok(
        &mut db,
        "INSERT INTO t VALUES (1, 7), (2, 7), (3, 7), (4, 8);",
    );
    run_ok(&mut db, "CREATE INDEX idx_t_b ON t (b);");

    assert_eq!(select_count(&mut db, "select * from t where b = 7;"), 3);
    assert_eq!(select_count(&mut db, "select * from t where b = 8;"), 1);
}

#[test]
fn composite_index_lookup() {
    let mut db = TestDB::new();
    run_ok(
        &mut db,
        "CREATE TABLE t (a INT, b INT, c INT, primary key (a));",
    );
    run_ok(
        &mut db,
        "INSERT INTO t VALUES (1, 1, 10), (2, 1, 20), (3, 2, 10);",
    );
    run_ok(&mut db, "CREATE INDEX idx_t_bc ON t (b, c);");

    // Full composite match uses the index; a partial (b-only) predicate
    // falls back to a normal scan+filter, both must be correct.
    assert_eq!(
        select_count(&mut db, "select * from t where b = 1 and c = 10;"),
        1
    );
    assert_eq!(select_count(&mut db, "select * from t where b = 1;"), 2);
}
