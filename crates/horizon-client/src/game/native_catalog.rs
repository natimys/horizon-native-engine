//! Projects the public runtime UI snapshot into Horizon's existing Mods screen.
use horizon_client::{font::Font, mod_list::CatalogMod, text::Text};
use horizon_native_api::ModSide;
use horizon_native_ui::{
    DependencyStatus, LOADING_MODEL, ModListView, REBUILD_NOTICE, TRUST_NOTICE,
};

pub(super) fn rows(view: Option<ModListView>, font: &Font) -> Vec<CatalogMod> {
    let Some(view) = view else { return Vec::new() };
    view.entries()
        .iter()
        .map(|entry| {
            let meta = entry.metadata;
            let side = match meta.side {
                ModSide::Common => "Common",
                ModSide::Client => "Client",
                ModSide::Server => "Server",
            };
            let mut details = vec![
                meta.name.to_owned(),
                format!("ID: {}", meta.id),
                format!("Version: {}", meta.version),
                format!("Loading: {LOADING_MODEL}"),
                format!("Side: {side}"),
                format!(
                    "Host selection: {}",
                    if entry.selected_for_host {
                        "Selected"
                    } else {
                        "Inactive on this host"
                    }
                ),
                // This is a runtime-wide state, never a fabricated per-mod result.
                format!("Runtime state: {:?}", view.runtime_state()),
                format!("Native API: {}", meta.api_version),
                format!("Authors: {}", meta.authors.join(", ")),
                format!("License: {}", meta.license),
                meta.description.to_owned(),
            ];
            if entry.dependencies.is_empty() {
                details.push("Dependencies: none".into());
            }
            for dependency in &entry.dependencies {
                let requirement = dependency.requirement;
                let status = match dependency.status {
                    DependencyStatus::Present {
                        selected_for_host: true,
                    } => "present, selected for host",
                    DependencyStatus::Present {
                        selected_for_host: false,
                    } => "present, inactive on host",
                    DependencyStatus::OptionalAbsent => "absent (optional)",
                };
                details.push(format!(
                    "Dependency: {} = {} ({}): {status}",
                    requirement.id,
                    requirement.version,
                    if requirement.optional {
                        "optional"
                    } else {
                        "required"
                    }
                ));
            }
            details.extend([REBUILD_NOTICE.into(), TRUST_NOTICE.into()]);
            CatalogMod {
                id: meta.id.into(),
                title: format!("{} {}", meta.name, meta.version),
                loading_model: LOADING_MODEL.into(),
                details: details
                    .into_iter()
                    .flat_map(|line| {
                        font.split(&Text::literal(line), 300.0)
                            .into_iter()
                            .map(|line| line.plain())
                    })
                    .collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_native_api::{API_VERSION, Dependency, ModMetadata, NativeMod};
    use horizon_native_runtime::{HostSide, Runtime, State};

    static ALPHA: ModMetadata = ModMetadata {
        id: "alpha",
        name: "Альфа",
        version: "1.2.3",
        description: "Длинное описание для проверки переноса текста: версия, авторы и зависимости должны читаться без обрезки даже при длинном локализованном названии и описании мода.",
        authors: &["A", "B"],
        license: "MIT",
        api_version: API_VERSION,
        side: ModSide::Common,
        dependencies: &[],
    };
    static BETA: ModMetadata = ModMetadata {
        id: "beta",
        name: "Beta",
        side: ModSide::Server,
        dependencies: &[
            Dependency {
                id: "alpha",
                version: "1.2.3",
                optional: false,
            },
            Dependency {
                id: "missing",
                version: "1",
                optional: true,
            },
        ],
        ..ALPHA
    };
    struct Example(&'static ModMetadata);
    impl NativeMod for Example {
        fn metadata(&self) -> &'static ModMetadata {
            self.0
        }
    }
    #[test]
    fn two_public_mods_project_metadata_dependencies_and_host_selection() {
        let atlas = horizon_client::hud::HudAtlas::build(std::path::Path::new(
            "/nonexistent/native-ui-assets",
        ))
        .unwrap();
        let empty = Runtime::for_side(Vec::new(), HostSide::Client).unwrap();
        assert!(rows(ModListView::for_runtime(&empty), &atlas.font).is_empty());
        assert!(rows(None, &atlas.font).is_empty());
        for side in [HostSide::Client, HostSide::Combined, HostSide::Server] {
            let mut runtime = Runtime::for_side(
                vec![Box::new(Example(&BETA)), Box::new(Example(&ALPHA))],
                side,
            )
            .unwrap();
            runtime.initialize().unwrap();
            let entries = rows(ModListView::for_runtime(&runtime), &atlas.font);
            assert_eq!(runtime.state(), State::Running);
            if side == HostSide::Server {
                assert!(entries.is_empty());
                continue;
            }
            assert_eq!(
                entries.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
                ["alpha", "beta"]
            );
            assert_eq!(entries[0].title, "Альфа 1.2.3");
            let text = entries[1].details.join(" ");
            for expected in [
                "alpha = 1.2.3",
                "missing = 1",
                "absent (optional)",
                "Runtime state: Running",
                "License: MIT",
                "Native API: 1",
            ] {
                assert!(text.contains(expected), "{expected}: {text}");
            }
            assert_eq!(
                text.contains("Host selection: Inactive"),
                side == HostSide::Client
            );
            for line in entries.iter().flat_map(|m| &m.details) {
                assert!(atlas.font.width(line) <= 300.0, "{line}");
            }
            assert_eq!(
                rows(ModListView::for_runtime(&runtime), &atlas.font),
                entries
            );
        }
    }
}
