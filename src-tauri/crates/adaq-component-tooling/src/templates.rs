use std::{
    fs,
    path::{Path, PathBuf},
};

use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentTemplate {
    Factor,
    Strategy,
    ComposedStrategy,
    Model,
}

impl ComponentTemplate {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "factor" => Ok(Self::Factor),
            "strategy" => Ok(Self::Strategy),
            "model" => Ok(Self::Model),
            _ => Err("Component kind must be factor, strategy, or model".into()),
        }
    }

    pub fn composed_strategy() -> Self {
        Self::ComposedStrategy
    }

    fn name(self) -> &'static str {
        match self {
            Self::Factor => "factor",
            Self::Strategy | Self::ComposedStrategy => "strategy",
            Self::Model => "model",
        }
    }
}

pub fn create_project(
    kind: ComponentTemplate,
    name: &str,
    parent: &Path,
    sdk_path: Option<&Path>,
) -> Result<PathBuf, String> {
    validate_name(name)?;
    let root = parent.join(name);
    if root.exists() {
        return Err(format!("Project already exists: {}", root.display()));
    }
    fs::create_dir_all(root.join("src")).map_err(string)?;
    let dependency = match sdk_path {
        Some(path) => format!(
            "adaq-component-sdk = {{ path = \"{}\", features = [\"{}\"] }}",
            path.to_string_lossy()
                .replace('\\', "/")
                .replace('"', "\\\""),
            kind.name()
        ),
        None => format!(
            "adaq-component-sdk = {{ version = \"={}\", features = [\"{}\"] }}",
            adaq_component_sdk::SDK_VERSION,
            kind.name()
        ),
    };
    let display_name = name
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let component_id = Uuid::new_v4().to_string();
    let render = |template: &str| {
        template
            .replace("{{name}}", name)
            .replace("{{display_name}}", &display_name)
            .replace("{{component_id}}", &component_id)
            .replace("{{sdk_dependency}}", &dependency)
            .replace("{{sdk_version}}", adaq_component_sdk::SDK_VERSION)
            .replace("{{artifact_sha256}}", &"0".repeat(64))
    };
    let source = match kind {
        ComponentTemplate::Factor => include_str!("../templates/factor/lib.rs"),
        ComponentTemplate::Strategy => include_str!("../templates/strategy/lib.rs"),
        ComponentTemplate::ComposedStrategy => {
            include_str!("../templates/strategy-composed/lib.rs")
        }
        ComponentTemplate::Model => include_str!("../templates/model/lib.rs"),
    };
    let manifest = match kind {
        ComponentTemplate::Factor => include_str!("../templates/factor/manifest.json"),
        ComponentTemplate::Strategy => include_str!("../templates/strategy/manifest.json"),
        ComponentTemplate::ComposedStrategy => {
            include_str!("../templates/strategy-composed/manifest.json")
        }
        ComponentTemplate::Model => include_str!("../templates/model/manifest.json"),
    };
    fs::write(
        root.join("Cargo.toml"),
        render(include_str!("../templates/Cargo.toml.template")),
    )
    .map_err(string)?;
    fs::write(root.join("src/lib.rs"), render(source)).map_err(string)?;
    fs::write(root.join("manifest.json"), render(manifest)).map_err(string)?;
    fs::write(
        root.join("README.md"),
        render(include_str!("../templates/README.md")),
    )
    .map_err(string)?;
    Ok(root)
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.starts_with('-')
        || name.ends_with('-')
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        Err("Project name must use lowercase ASCII letters, digits, and interior hyphens".into())
    } else {
        Ok(())
    }
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_both_project_kinds_without_overwriting() {
        let root = tempfile::tempdir().unwrap();
        let sdk = Path::new("/tmp/adaq-component-sdk");
        for (kind, name) in [
            (ComponentTemplate::Factor, "factor"),
            (ComponentTemplate::Strategy, "strategy"),
            (ComponentTemplate::ComposedStrategy, "composed-strategy"),
            (ComponentTemplate::Model, "model"),
        ] {
            let project = create_project(kind, name, root.path(), Some(sdk)).unwrap();
            let cargo = fs::read_to_string(project.join("Cargo.toml")).unwrap();
            let manifest = fs::read_to_string(project.join("manifest.json")).unwrap();
            assert!(cargo.contains("adaq-component-sdk"));
            assert!(manifest.contains("\"sdkVersion\": \"0.1.0\""));
            if matches!(
                kind,
                ComponentTemplate::Strategy | ComponentTemplate::ComposedStrategy
            ) {
                let manifest: crate::ComponentManifest = serde_json::from_str(&manifest).unwrap();
                assert_eq!(
                    crate::strategy_architecture(&manifest),
                    Some(if kind == ComponentTemplate::Strategy {
                        crate::StrategyArchitecture::SignalDriven
                    } else {
                        crate::StrategyArchitecture::Composed
                    })
                );
            }
            assert!(create_project(kind, name, root.path(), Some(sdk)).is_err());
        }
    }
}
