#[test]
fn plugin_version_matches_cargo_package() {
    let plugin: toml::Value = toml::from_str(include_str!("../herdr-plugin.toml")).unwrap();
    assert_eq!(plugin["version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
}
