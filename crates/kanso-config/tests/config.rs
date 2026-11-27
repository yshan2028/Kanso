use std::collections::BTreeMap;
use std::fs;

use kanso_config::{
    ConfigError, ConfigLayer, EffectiveEntry, KansoConfig, PriceOverride, effective, load_file,
    save_user,
};

fn policy(value: &str) -> KansoConfig {
    KansoConfig {
        routing_policy: Some(value.into()),
        ..KansoConfig::default()
    }
}

fn entry(key: &str, value: &str, layer: ConfigLayer) -> EffectiveEntry {
    EffectiveEntry {
        key: key.into(),
        value: value.into(),
        layer,
    }
}

fn price(input: f64) -> PriceOverride {
    PriceOverride {
        input,
        cache_write: 2.0,
        cache_read: 0.25,
        output: 4.0,
    }
}

#[test]
fn each_of_the_six_layers_wins_over_all_lower_layers() {
    use ConfigLayer::*;

    let mut layers = vec![
        (Repo, policy("repo")),
        (Builtin, policy("builtin")),
        (User, policy("user")),
        (CliFlag, policy("cli")),
        (Import, policy("import")),
        (Task, policy("task")),
    ];
    for (layer, value) in [
        (CliFlag, "cli"),
        (Task, "task"),
        (Repo, "repo"),
        (User, "user"),
        (Import, "import"),
        (Builtin, "builtin"),
    ] {
        assert_eq!(
            effective(&layers),
            [entry("routing.worker.policy", value, layer)]
        );
        layers.retain(|(candidate, _)| *candidate != layer);
    }
    assert!(effective(&layers).is_empty());
}

#[test]
fn keys_inherit_independently_and_explicit_false_and_empty_values_win() {
    use ConfigLayer::*;

    let layers = [
        (
            Builtin,
            KansoConfig {
                require_cross_family: Some(true),
                provider_order: Some(vec!["builtin-provider".into()]),
                provider_disabled: Some(vec!["provider-c".into(), "provider-d".into()]),
                ..KansoConfig::default()
            },
        ),
        (
            User,
            KansoConfig {
                provider_order: Some(vec!["provider-b".into(), "provider-a".into()]),
                provider_disabled: Some(Vec::new()),
                ..KansoConfig::default()
            },
        ),
        (CliFlag, policy("selected")),
        (
            Repo,
            KansoConfig {
                require_cross_family: Some(false),
                ..KansoConfig::default()
            },
        ),
    ];
    assert_eq!(
        effective(&layers),
        [
            entry("routing.worker.policy", "selected", CliFlag),
            entry("routing.reviewer.require_cross_family", "false", Repo),
            entry("providers.order", "provider-b, provider-a", User),
            entry("providers.disabled", "", User),
        ]
    );
    assert_eq!(
        effective(&layers[..1])
            .into_iter()
            .find(|entry| entry.key == "providers.disabled"),
        Some(entry(
            "providers.disabled",
            "provider-c, provider-d",
            Builtin
        ))
    );
}

#[test]
fn equal_layers_use_the_first_setting_for_each_key() {
    use ConfigLayer::*;

    let layers = [
        (Task, policy("first")),
        (Repo, policy("repo")),
        (
            Task,
            KansoConfig {
                provider_order: Some(vec!["provider-a".into()]),
                ..policy("second")
            },
        ),
    ];
    assert_eq!(
        effective(&layers),
        [
            entry("routing.worker.policy", "first", Task),
            entry("providers.order", "provider-a", Task),
        ]
    );
}

#[test]
fn prices_merge_by_pattern() {
    use ConfigLayer::*;

    let layers = [
        (
            Builtin,
            KansoConfig {
                pricing: Some(BTreeMap::from([
                    ("model-a.*".into(), price(1.0)),
                    ("model-b".into(), price(3.0)),
                ])),
                ..KansoConfig::default()
            },
        ),
        (
            User,
            KansoConfig {
                pricing: Some(BTreeMap::from([
                    ("model-a.*".into(), price(5.0)),
                    ("model-c".into(), price(6.0)),
                ])),
                ..KansoConfig::default()
            },
        ),
        (
            CliFlag,
            KansoConfig {
                pricing: Some(BTreeMap::new()),
                ..KansoConfig::default()
            },
        ),
    ];
    assert_eq!(
        effective(&layers),
        [
            entry("pricing.model-a.*", "in=$5 cw=$2 cr=$0.25 out=$4", User),
            entry("pricing.model-c", "in=$6 cw=$2 cr=$0.25 out=$4", User),
            entry("pricing.model-b", "in=$3 cw=$2 cr=$0.25 out=$4", Builtin),
        ]
    );
}

#[test]
fn missing_file_returns_a_typed_error() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("missing.toml");
    let error = load_file(&path).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ConfigError>(),
        Some(ConfigError::NotFound { path: missing, .. }) if missing == &path
    ));
    Ok(())
}

#[test]
fn invalid_toml_returns_a_parse_error() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("invalid.toml");
    fs::write(&path, "provider_order = [")?;
    let error = load_file(&path).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<ConfigError>(),
        Some(ConfigError::Parse { path: invalid, .. }) if invalid == &path
    ));
    Ok(())
}

#[test]
fn reading_and_writing_a_directory_return_distinct_io_errors() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let read_error = load_file(dir.path()).unwrap_err();
    assert!(matches!(
        read_error.downcast_ref::<ConfigError>(),
        Some(ConfigError::Read { path, .. }) if path == dir.path()
    ));
    let write_error = save_user(dir.path(), &KansoConfig::default()).unwrap_err();
    assert!(matches!(
        write_error.downcast_ref::<ConfigError>(),
        Some(ConfigError::Write { path, .. }) if path == dir.path()
    ));
    Ok(())
}

#[test]
fn user_config_round_trips_with_nested_directories_and_overwrite() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("user/settings/config.toml");
    let config = KansoConfig {
        routing_policy: Some("均衡".into()),
        require_cross_family: Some(false),
        provider_order: Some(vec!["provider-a".into(), "provider-b".into()]),
        provider_disabled: Some(Vec::new()),
        pricing: Some(BTreeMap::from([("model.v1-*".into(), price(1.5))])),
    };
    save_user(&path, &config)?;
    assert_eq!(load_file(&path)?, config);

    let empty = KansoConfig::default();
    save_user(&path, &empty)?;
    assert_eq!(load_file(&path)?, empty);
    Ok(())
}
