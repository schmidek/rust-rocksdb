mod util;

use rocksdb::{Options, ReadOptions, DB};
use util::DBPath;

#[test]
fn verify_checksum_test() {
    let n = DBPath::new("_rust_rocksdb_verify_checksum_test");
    let mut opts = Options::default();
    opts.create_if_missing(true);
    {
        let db = DB::open(&opts, &n).unwrap();
        for i in 0..1000 {
            db.put(format!("key{i}"), format!("value{i}")).unwrap();
        }
        db.flush().unwrap();
        db.verify_checksum().unwrap();
        db.verify_checksum_opt(&ReadOptions::default()).unwrap();
    }

    // Flip a byte in the first data block of the sst file
    let sst = {
        let db = DB::open(&opts, &n).unwrap();
        let files = db.live_files().unwrap();
        assert_eq!(files.len(), 1);
        db.path().join(files[0].name.trim_start_matches('/'))
    };
    let mut bytes = std::fs::read(&sst).unwrap();
    bytes[10] ^= 0xff;
    std::fs::write(&sst, bytes).unwrap();

    let mut opts = opts.clone();
    opts.set_skip_stats_update_on_db_open(true);
    opts.set_max_open_files(20);
    opts.set_paranoid_checks(false);
    let db = DB::open_for_read_only(&opts, &n, false).unwrap();
    let err = db.verify_checksum().unwrap_err().into_string();
    assert!(err.contains("Corruption"), "{err}");
    assert!(
        err.contains(sst.file_name().unwrap().to_str().unwrap()),
        "{err}"
    );
}
