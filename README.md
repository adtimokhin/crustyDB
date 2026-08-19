# CrustyDB

CrustyDB is a relational database management system built from scratch in Rust: a slotted-page storage engine with a disk-backed buffer pool, a B+Tree indexing engine, Volcano-model query execution, and a real cost-based query optimizer.

## Features

- **Storage**: a slotted-page format (fixed-size 4KB pages storing variable-length records) built on top of a buffer pool that reads and writes pages to disk via `pread`/`pwrite`, with page-level latching for concurrent access.
- **Indexing**: a B+Tree engine (`src/index`) built on the same slotted-page infrastructure, supporting single-column and composite keys, non-unique (secondary) indexes with duplicate keys, and `CREATE INDEX`. Index entries are maintained automatically on `INSERT`, `UPDATE`, and `DELETE`.
- **Query execution**: a Volcano-style (iterator/pull-based) execution engine with sequential scan, index scan, filter, project, nested-loop join, hash join, sort-merge join, hash-based group-by/aggregate, sort, update, and delete operators.
- **Query optimization**: a cost-based optimizer (`CascadesOptimizer`) that builds a memo of candidate physical plans and picks the cheapest using real cardinality/selectivity statistics (`ReservoirStatManager`) instead of fixed heuristics — for example, choosing between an index scan and a full scan based on estimated selectivity, and between a hash join and a nested-loop join based on estimated input sizes.
- **SQL surface**: `CREATE TABLE`, `CREATE INDEX`, `INSERT`, `SELECT` (with `WHERE`, joins, `GROUP BY`/aggregates), `UPDATE`, and `DELETE`, via a client/server architecture with a `psql`-like CLI client.

## Usage

Make sure you have Rust > 1.81.0. Updating the Rust toolchain is easy:

```bash
$ rustup update
```

You can then check the version:

```bash
$ rustc --version
```

## Building the project

To build the entire CrustyDB source code, run `cargo build`.

CrustyDB is set up as a workspace, with various modules/components broken out into separate packages/crates. To build a specific crate (for example `common`), use `cargo build -p common`. If a crate depends on another (e.g. `heapstore` depends on `common` and `txn_manager`), those crates will be built as part of the process.

These crates are:
- `cli-crusty`: a command line interface client binary application that can connect and issue commands/queries to a running CrustyDB server.
- `common`: shared data structures and logical components needed by everything in CrustyDB — tables, errors, logical/physical query plans, ids, test utilities, etc. Organized into modules split by physical layout, shared query execution operations and representations, traits (interfaces), and utilities.
- `index`: the B+Tree indexing engine, plus the `IndexManager` that owns live indexes and maintains them on writes.
- `optimizer`: query optimization — a memo-based, cost-driven optimizer (`CascadesOptimizer`) and a statistics-backed cost model (`CardinalityCostModel`), alongside a simpler structural fallback (`MockOptimizer`) used for tests.
- `queryexe`: responsible for executing queries. Contains the operator implementations as well as the execution code for the Volcano-style execution engine.
- `server`: the binary crate for running a CrustyDB server. Connects all modules (outside the client) together.
- `storage`: the storage managers for the database, including a buffer pool. Only one storage manager is used at a time:
  - `heapstore`: the primary storage manager, storing data in slotted-page heap files.
  - `memstore`: a simpler storage manager that keeps everything in memory, persisting to files via serde on shutdown and reloading them on startup.
- `txn_manager`: transaction management. Currently a no-op stub — no isolation/locking is implemented yet (a natural next milestone for this project).
- `utilities`: shared utilities used by performance benchmarks.

There's also an `e2e-tests` crate outside the main workspace, used for end-to-end testing (e.g. sending SQL to the server and checking the response) and for `criterion` benchmarks over the full stack.

## Tests

Most crates have tests that can be run using `cargo test`. Like building, you can run tests for a single crate with `cargo test -p common`. Note that tests build/compile code in the tests modules, so you may encounter build errors here that don't show up in a regular build.

### Running an ignored test

Some longer tests are set to be ignored by default. To run them: `cargo test -- --ignored`

### Benchmarks

`e2e-tests` includes `criterion` benchmarks (`cargo bench` from that directory) covering storage manager throughput, page-level operations, filters, joins, and index scans.

## Logging

CrustyDB uses the [env_logger](https://docs.rs/env_logger/0.8.2/env_logger/) crate for logging messages. Per the docs on the `log` crate:
```
The basic use of the log crate is through the five logging macros: error!, warn!, info!, debug! and trace!
where error! represents the highest-priority log messages and trace! the lowest.
The log messages are filtered by configuring the log level to exclude messages with a lower priority.
Each of these macros accept format strings similarly to println!.
```

The logging level is set by an environment variable, `RUST_LOG`. The easiest way to set the level is to set it in the same command you're running. E.g.: `RUST_LOG=debug cargo run --bin server`. When running unit tests, logging output is suppressed and the logger isn't initialized by default, so to see logging in a test:
- Make sure the test calls `init()` (defined in `common::testutils`), which initializes the logger. It's safe to call multiple times.
- Tell cargo not to capture output. For example, at DEBUG level: `RUST_LOG=debug cargo test -- --nocapture [opt_test_name]` (note the `--` before `--nocapture`).

Examples:
```
RUST_LOG=debug cargo run --bin server
RUST_LOG=debug cargo test
RUST_LOG=debug cargo test -- --nocapture [test_name]
```

The log level can also be set programmatically, in the first line of `main()` in the server crate (defaults to DEBUG).

### Connecting to a Database

This is the basic process for starting a database and connecting to it via the CLI client.

1. Start a server:

    ```
    $ cargo run --bin server
    ```

2. Start a client with logging enabled to see output:

    ```
    $ RUST_LOG=info cargo run --bin cli-crusty
    ```

### Client Commands

CrustyDB emulates `psql` commands.

Command | Functionality
---------|--------------
`\r [DATABASE]` | Creates a new database, DATABASE
`\c [DATABASE]` | Connects to DATABASE
`\i [PATH] [TABLE_NAME]` | Imports a CSV file at PATH and saves it to TABLE_NAME in whatever database the client is currently connected to.
`\l` | List the name of all databases present on the server.
`\dt` | List the name of all tables present on the current database.
`\generate [CSV_NAME] [NUMBER_OF_RECORDS]` | Generate a test CSV for a sample schema.
`\reset` | Deletes all data and state for all databases on the server.
`\close` | Closes the current client, but leaves the database server running.
`\shutdown` | Shuts down the database server cleanly (allows the DB to gracefully exit).

The client also handles SQL queries and statements directly.

## End to End Example

After compiling the database, start a server and a client instance.

To start the CrustyDB server:

```
$ cargo run --bin server
```

and to start the client:

```
$ cargo run --bin cli-crusty
```

Now, from the client, you can interact with the server. Create a database named `testdb`:

```
[crustydb]>> \r testdb
```

Then connect to the newly created database:

```
[crustydb]>> \c testdb
```

Create a table with 2 integer columns, named `a` and `b`:

```
[crustydb]>> CREATE TABLE test (a INT, b INT, primary key (a));
```

The table exists but doesn't contain any data yet. The repository includes a sample CSV file (`data.csv`) you can import:

```
[crustydb]>> \i <PATH>/data.csv test
```

(Replace `<PATH>` with the path to wherever `data.csv` lives in the repository.)

Now run some SQL against it:

```
[crustydb]>> SELECT a FROM test;
[crustydb]>> SELECT sum(a), sum(b) FROM test;
```

Create a secondary index and confirm it's used automatically for a selective lookup:

```
[crustydb]>> CREATE INDEX idx_test_b ON test (b);
[crustydb]>> SELECT * FROM test WHERE b = 2;
```

Update and delete rows — the index above stays consistent automatically:

```
[crustydb]>> UPDATE test SET b = 100 WHERE a = 1;
[crustydb]>> DELETE FROM test WHERE a = 2;
```

As you follow through this example, it's worth watching the server's log messages (`RUST_LOG=debug cargo run --bin server`) — that's a good way to see the lifecycle of query planning and execution in CrustyDB, including which physical plan the optimizer picked.

### Client Scripts

The client can run a series of commands/queries from a text file. Each command or query must be separated by a `;` (even commands that wouldn't normally need one when using the CLI interactively). To use a script, pass `-- -s [script file]`:

```
cargo run -p cli-crusty -- -s [script file]
```

### Shutdown

Shutting down the server is not automatic. You need to manually shut it down with `\shutdown` from the client, or Ctrl-C in the client terminal (Ctrl-D disconnects the client but leaves the server running). This allows for a clean shutdown of the server and the database.

A non-clean shutdown will likely leave the database in an inconsistent state. You'll need to clean the database by removing the `crusty_data` directory and re-running the server (`rm -rf crusty_data/`).

## Debugging Rust Programs

Debugging is a crucial skill for any software developer. If you write software, your software will contain bugs — debugging is the process of finding those bugs so you can fix them.

### Debuggers

There are tools to help you debug software called debuggers. In the C/C++ world, `gdb` and `lldb` are the two popular ones: `gdb` is typically used on Linux, `lldb` on macOS, and either on Windows depending on your setup.

### Debuggers in the IDE

If you use Visual Studio Code, you can use a Rust debugger (based on `gdb` or `lldb` depending on platform) — instructions are easy to find online.

JetBrains' [CLion](https://www.jetbrains.com/clion/) has solid Rust debugger support with the Rust plugin installed; it's not free, but offers free licenses for students and open-source maintainers.

### Alternative ways of debugging programs

Beyond a full debugger, Rust's `println!()` macro (and CrustyDB's logging, see above) and the standard library's `dbg!()` macro (which prints an expression's value along with the file/line it's found at) are simple, effective ways to inspect program behavior — especially useful for quick checks even when a full debugger is available.
