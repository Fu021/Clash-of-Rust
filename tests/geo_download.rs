use clash_of_rust::assets::{download_geo, verify_geo};

#[tokio::test]
#[ignore = "Explicit online test; GEO_TEST_PROXY may specify a local mixed proxy port"]
async fn official_geo_downloads_pass_checksums() {
    let temporary = tempfile::tempdir().unwrap();
    let proxy = std::env::var("GEO_TEST_PROXY")
        .ok()
        .map(|p| p.parse::<u16>().unwrap());
    let downloaded = download_geo(temporary.path(), proxy).await.unwrap();
    assert_eq!(downloaded.files.len(), 4);
    let checked = verify_geo(temporary.path()).unwrap();
    assert_eq!(checked.version, downloaded.version);
    assert!(
        checked
            .files
            .iter()
            .all(|file| file.size > 1024 && file.sha256.len() == 64)
    );
}
