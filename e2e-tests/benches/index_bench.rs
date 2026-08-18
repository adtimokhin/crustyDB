use cli_crusty::Response;
use common::QueryResult;
use criterion::{criterion_group, criterion_main, Criterion};
use e2e_tests::test_db::TestDB;

// Measuring commands should not modify the database itself
fn bench_template(
    c: &mut Criterion,
    bench_name: &str,
    setup_commands: Vec<&str>,
    measuring_commands: Vec<(&str, usize)>,
) {
    let mut db = TestDB::new();
    for command in setup_commands {
        let res = db.send_command(command.as_bytes());
        assert_eq!(res.len(), 1);
        if matches!(
            res[0],
            Response::SystemErr(_) | Response::QuietErr | Response::QueryExecutionError(_)
        ) {
            panic!("Error in setup command: {}", command);
        }
    }

    c.bench_function(bench_name, |b| {
        b.iter(|| {
            for (command, result_size) in &measuring_commands {
                let res = db.send_command(command.as_bytes());
                assert_eq!(res.len(), 1);
                match &res[0] {
                    Response::QueryResult(QueryResult::Select { result, .. }) => {
                        assert_eq!(result.len(), *result_size);
                    }
                    _ => {
                        panic!("Error in measuring command: {}", command);
                    }
                }
            }
        });
    });
}

/// Point lookup on a non-PK column, no index: falls back to a full SeqScan + Filter.
fn bench_point_lookup_seqscan(c: &mut Criterion) {
    let setup_commands = vec![
        "CREATE TABLE testA (a INT, b INT, primary key (a));",
        "\\i csv/large_data.csv testA",
    ];
    let measuring_commands = vec![("select * from testA where b = 500", 1)];
    bench_template(c, "point_lookup_seqscan_1k_rows", setup_commands, measuring_commands);
}

/// Same table, same query, but with a B+Tree index on `b`: planner rewrites
/// Scan+Select into an IndexScan (see Translator::process_where).
fn bench_point_lookup_indexscan(c: &mut Criterion) {
    let setup_commands = vec![
        "CREATE TABLE testA (a INT, b INT, primary key (a));",
        "\\i csv/large_data.csv testA",
        "CREATE INDEX idx_testa_b ON testA (b);",
    ];
    let measuring_commands = vec![("select * from testA where b = 500", 1)];
    bench_template(c, "point_lookup_indexscan_1k_rows", setup_commands, measuring_commands);
}

criterion_group! {
    name = index_bench;
    config = Criterion::default().sample_size(10);
    targets = bench_point_lookup_seqscan, bench_point_lookup_indexscan,
}

criterion_main!(index_bench);
