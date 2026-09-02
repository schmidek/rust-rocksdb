// Copyright 2020 Tyler Neely
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

mod util;

use std::sync::{Arc, Mutex};

use pretty_assertions::assert_eq;
use rocksdb::{GetMergeOperandsOptions, MergeOperands, Options, ReadOptions, DB};
use util::DBPath;

fn concat_merge(
    _new_key: &[u8],
    existing_val: Option<&[u8]>,
    operands: &MergeOperands,
) -> Option<Vec<u8>> {
    let mut result = existing_val.map_or_else(Vec::new, Vec::from);
    for op in operands {
        result.extend_from_slice(op);
    }
    Some(result)
}

fn db_options() -> Options {
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.set_merge_operator_associative("test operator", concat_merge);
    opts
}

/// Opens a DB with three merge operands stored under `mergekey`.
fn db_with_operands(path: &DBPath) -> DB {
    let db = DB::open(&db_options(), path).unwrap();
    for op in [b"op1", b"op2", b"op3"] {
        db.merge(b"mergekey", op).unwrap();
    }
    db
}

fn operands_options(expected_max_number_of_operands: i32) -> GetMergeOperandsOptions {
    let mut opts = GetMergeOperandsOptions::default();
    opts.set_expected_max_number_of_operands(expected_max_number_of_operands);
    opts
}

#[track_caller]
fn assert_operands(operands: &[rocksdb::DBPinnableSlice], want: &[&[u8]]) {
    let got = operands.iter().map(|op| op.to_vec()).collect::<Vec<_>>();
    let want = want.iter().map(|op| op.to_vec()).collect::<Vec<_>>();
    assert_eq!(got, want);
}

#[test]
fn test_expected_max_number_of_operands() {
    let mut opts = GetMergeOperandsOptions::default();
    assert_eq!(opts.expected_max_number_of_operands(), 0);

    opts.set_expected_max_number_of_operands(3);
    assert_eq!(opts.expected_max_number_of_operands(), 3);
}

#[test]
fn test_get_merge_operands() {
    let path = DBPath::new("_rust_rocksdb_get_merge_operands");
    {
        let db = db_with_operands(&path);
        let operands = db
            .get_merge_operands(b"mergekey", &operands_options(3))
            .unwrap();

        // Operands come back in insertion order, oldest first.
        assert_operands(&operands, &[b"op1", b"op2", b"op3"]);
    }
}

#[test]
fn test_get_merge_operands_more_room_than_needed() {
    let path = DBPath::new("_rust_rocksdb_get_merge_operands_more_room_than_needed");
    {
        let db = db_with_operands(&path);
        let operands = db
            .get_merge_operands(b"mergekey", &operands_options(8))
            .unwrap();

        assert_operands(&operands, &[b"op1", b"op2", b"op3"]);
    }
}

#[test]
fn test_get_merge_operands_not_enough_room() {
    let path = DBPath::new("_rust_rocksdb_get_merge_operands_not_enough_room");
    {
        let db = db_with_operands(&path);

        // Fewer slots than the key has operands: an error, and no operand read.
        assert!(db
            .get_merge_operands(b"mergekey", &operands_options(2))
            .is_err());

        // The default options have room for no operand at all.
        assert!(db
            .get_merge_operands(b"mergekey", &GetMergeOperandsOptions::default())
            .is_err());
    }
}

#[test]
fn test_get_merge_operands_absent_key() {
    let path = DBPath::new("_rust_rocksdb_get_merge_operands_absent_key");
    {
        let db = db_with_operands(&path);

        // A key without merge operands yields no operand and no error, even
        // when there is no room for operands.
        assert!(db
            .get_merge_operands(b"absentkey", &operands_options(3))
            .unwrap()
            .is_empty());
        assert!(db
            .get_merge_operands(b"absentkey", &GetMergeOperandsOptions::default())
            .unwrap()
            .is_empty());
    }
}

#[test]
fn test_get_merge_operands_opt() {
    let path = DBPath::new("_rust_rocksdb_get_merge_operands_opt");
    {
        let db = db_with_operands(&path);

        let snapshot = db.snapshot();
        let mut readopts = ReadOptions::default();
        readopts.set_snapshot(&snapshot);

        db.merge(b"mergekey", b"op4").unwrap();

        // The snapshot predates the fourth operand.
        let operands = db
            .get_merge_operands_opt(b"mergekey", &readopts, &operands_options(8))
            .unwrap();
        assert_operands(&operands, &[b"op1", b"op2", b"op3"]);

        let operands = db
            .get_merge_operands(b"mergekey", &operands_options(8))
            .unwrap();
        assert_operands(&operands, &[b"op1", b"op2", b"op3", b"op4"]);
    }
}

#[test]
fn test_get_merge_operands_cf() {
    let path = DBPath::new("_rust_rocksdb_get_merge_operands_cf");
    {
        let mut db = DB::open(&db_options(), &path).unwrap();
        db.create_cf("cf1", &db_options()).unwrap();
        let cf1 = db.cf_handle("cf1").unwrap();

        for op in [b"op1", b"op2", b"op3"] {
            db.merge_cf(&cf1, b"mergekey", op).unwrap();
        }
        // The same key in the default column family gets a different operand,
        // so a column family mix-up cannot go unnoticed.
        db.merge(b"mergekey", b"default").unwrap();

        let operands = db
            .get_merge_operands_cf(&cf1, b"mergekey", &operands_options(3))
            .unwrap();
        assert_operands(&operands, &[b"op1", b"op2", b"op3"]);

        let operands = db
            .get_merge_operands_cf_opt(
                &cf1,
                b"mergekey",
                &ReadOptions::default(),
                &operands_options(3),
            )
            .unwrap();
        assert_operands(&operands, &[b"op1", b"op2", b"op3"]);

        assert!(db
            .get_merge_operands_cf(&cf1, b"mergekey", &operands_options(2))
            .is_err());
        assert!(db
            .get_merge_operands_cf(&cf1, b"absentkey", &operands_options(3))
            .unwrap()
            .is_empty());
    }
}

#[test]
fn test_snapshot_get_merge_operands() {
    let path = DBPath::new("_rust_rocksdb_snapshot_get_merge_operands");
    {
        let db = db_with_operands(&path);
        let snapshot = db.snapshot();

        db.merge(b"mergekey", b"op4").unwrap();

        // The snapshot predates the fourth operand, whichever read options it
        // is handed.
        let operands = snapshot
            .get_merge_operands(b"mergekey", &operands_options(8))
            .unwrap();
        assert_operands(&operands, &[b"op1", b"op2", b"op3"]);

        let operands = snapshot
            .get_merge_operands_opt(b"mergekey", ReadOptions::default(), &operands_options(8))
            .unwrap();
        assert_operands(&operands, &[b"op1", b"op2", b"op3"]);

        // Fewer slots than the key has operands: an error, and no operand read.
        assert!(snapshot
            .get_merge_operands(b"mergekey", &operands_options(2))
            .is_err());

        // A key without merge operands yields no operand and no error.
        assert!(snapshot
            .get_merge_operands(b"absentkey", &operands_options(3))
            .unwrap()
            .is_empty());
    }
}

#[test]
fn test_snapshot_get_merge_operands_cf() {
    let path = DBPath::new("_rust_rocksdb_snapshot_get_merge_operands_cf");
    {
        let mut db = DB::open(&db_options(), &path).unwrap();
        db.create_cf("cf1", &db_options()).unwrap();
        let cf1 = db.cf_handle("cf1").unwrap();

        for op in [b"op1", b"op2", b"op3"] {
            db.merge_cf(&cf1, b"mergekey", op).unwrap();
        }
        // The same key in the default column family gets a different operand,
        // so a column family mix-up cannot go unnoticed.
        db.merge(b"mergekey", b"default").unwrap();

        let snapshot = db.snapshot();
        db.merge_cf(&cf1, b"mergekey", b"op4").unwrap();

        let operands = snapshot
            .get_merge_operands_cf(&cf1, b"mergekey", &operands_options(8))
            .unwrap();
        assert_operands(&operands, &[b"op1", b"op2", b"op3"]);

        let operands = snapshot
            .get_merge_operands_cf_opt(
                &cf1,
                b"mergekey",
                ReadOptions::default(),
                &operands_options(8),
            )
            .unwrap();
        assert_operands(&operands, &[b"op1", b"op2", b"op3"]);

        assert!(snapshot
            .get_merge_operands_cf(&cf1, b"mergekey", &operands_options(2))
            .is_err());
        assert!(snapshot
            .get_merge_operands_cf(&cf1, b"absentkey", &operands_options(3))
            .unwrap()
            .is_empty());
    }
}

#[test]
fn test_get_merge_operands_continue_cb() {
    let path = DBPath::new("_rust_rocksdb_get_merge_operands_continue_cb");
    {
        let db = db_with_operands(&path);

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut opts = operands_options(3);
        {
            let seen = Arc::clone(&seen);
            opts.set_continue_cb(move |operand: &[u8]| {
                seen.lock().unwrap().push(operand.to_vec());
                true
            });
        }

        // A callback that never stops the lookup reads every operand, from
        // newest to oldest, and the operands still come back in insertion order.
        let operands = db.get_merge_operands(b"mergekey", &opts).unwrap();
        assert_operands(&operands, &[b"op1", b"op2", b"op3"]);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [b"op3".to_vec(), b"op2".to_vec(), b"op1".to_vec()]
        );
    }
}

#[test]
fn test_get_merge_operands_continue_cb_stops_lookup() {
    let path = DBPath::new("_rust_rocksdb_get_merge_operands_continue_cb_stops_lookup");
    {
        let db = db_with_operands(&path);

        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut opts = operands_options(3);
        {
            let seen = Arc::clone(&seen);
            opts.set_continue_cb(move |operand: &[u8]| {
                seen.lock().unwrap().push(operand.to_vec());
                false
            });
        }

        // Stopping at the first operand the callback sees leaves only the
        // newest operand to return.
        let operands = db.get_merge_operands(b"mergekey", &opts).unwrap();
        assert_operands(&operands, &[b"op3"]);
        assert_eq!(seen.lock().unwrap().as_slice(), [b"op3".to_vec()]);

        // Replacing the callback replaces the operands the lookup stops at.
        let seen = Arc::new(Mutex::new(Vec::new()));
        {
            let seen = Arc::clone(&seen);
            opts.set_continue_cb(move |operand: &[u8]| {
                seen.lock().unwrap().push(operand.to_vec());
                operand != b"op2"
            });
        }
        let operands = db.get_merge_operands(b"mergekey", &opts).unwrap();
        assert_operands(&operands, &[b"op2", b"op3"]);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [b"op3".to_vec(), b"op2".to_vec()]
        );

        // Clearing the callback restores the read-everything behaviour.
        opts.clear_continue_cb();
        let operands = db.get_merge_operands(b"mergekey", &opts).unwrap();
        assert_operands(&operands, &[b"op1", b"op2", b"op3"]);
    }
}

#[test]
fn test_get_merge_operands_continue_cb_is_dropped() {
    let canary = Arc::new(());

    let mut opts = GetMergeOperandsOptions::default();
    {
        let canary = Arc::clone(&canary);
        opts.set_continue_cb(move |_: &[u8]| {
            let _ = &canary;
            true
        });
    }
    assert_eq!(Arc::strong_count(&canary), 2);

    // Setting a callback drops the previously set one...
    opts.set_continue_cb(|_: &[u8]| true);
    assert_eq!(Arc::strong_count(&canary), 1);

    {
        let canary = Arc::clone(&canary);
        opts.set_continue_cb(move |_: &[u8]| {
            let _ = &canary;
            true
        });
    }
    assert_eq!(Arc::strong_count(&canary), 2);

    // ...and so does clearing it.
    opts.clear_continue_cb();
    assert_eq!(Arc::strong_count(&canary), 1);

    {
        let canary = Arc::clone(&canary);
        opts.set_continue_cb(move |_: &[u8]| {
            let _ = &canary;
            true
        });
    }
    assert_eq!(Arc::strong_count(&canary), 2);

    // Dropping the options drops the callback it still holds.
    drop(opts);
    assert_eq!(Arc::strong_count(&canary), 1);
}
