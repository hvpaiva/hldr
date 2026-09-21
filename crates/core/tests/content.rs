//! The content shipped in the image must index, or the server panics on
//! boot and the deploy only fails at the health check.

use std::path::Path;

use hldr_core::Db;

#[tokio::test]
async fn repository_content_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("hldr.db")).await.unwrap();
    let content = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content");

    let report = hldr_core::index::sync(db.pool(), &content).await.unwrap();

    assert!(report.profile_updated);
    assert!(report.projects_upserted > 0);
}
