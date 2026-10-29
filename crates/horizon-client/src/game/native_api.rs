//! Generic ownership bridge for the public-API native path. Independent of WASM/mixins.
use super::execution_role::ExecutionRole;
use horizon_native_api::{ContentSnapshot, ModAction};
use horizon_native_runtime::{HostSide, Runtime};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

/// Drop also runs on startup/loop errors. Mods never receive Game or engine handles.
pub(super) struct NativeRuntime {
    pub(super) runtime: Runtime,
    pub(super) gameplay: super::native_gameplay::GameplayState,
}

fn host_side(role: ExecutionRole, server_thread: bool) -> HostSide {
    match role {
        ExecutionRole::Client if server_thread => HostSide::Client,
        ExecutionRole::Client => HostSide::Combined,
        ExecutionRole::IntegratedServer | ExecutionRole::DedicatedServer => HostSide::Server,
    }
}

fn output(actions: Vec<ModAction>) {
    for action in actions {
        match action {
            ModAction::Log { mod_id, message } => log::info!("[native:{mod_id}] {message}"),
        }
    }
}

impl NativeRuntime {
    pub(super) fn catalog(&self) -> Option<horizon_native_ui::ModListView> {
        horizon_native_ui::ModListView::for_runtime(&self.runtime)
    }

    #[cfg(test)]
    pub(super) fn start(role: ExecutionRole, server_thread: bool) -> super::Result<Self> {
        let mut owner = Self::prepare(role, server_thread)?;
        owner.initialize()?;
        Ok(owner)
    }

    fn prepare(role: ExecutionRole, server_thread: bool) -> super::Result<Self> {
        let runtime = horizon_native_pack::create_for_side(host_side(role, server_thread))?;
        let mut owner = Self {
            runtime,
            gameplay: Default::default(),
        };
        owner.runtime.freeze()?;
        Ok(owner)
    }

    fn initialize(&mut self) -> super::Result<()> {
        output(self.runtime.initialize()?);
        for metadata in self.runtime.metadata() {
            log::info!(
                "Native mod: {} {} ({})",
                metadata.name,
                metadata.version,
                metadata.id
            );
        }
        Ok(())
    }
}

/// Shared resource declarations, with the first registered instance retained
/// until Game setup. Each later Game owns its own side-scoped lifecycle.
pub(super) struct NativeResources {
    content: ContentSnapshot,
    prepared: Mutex<Option<NativeRuntime>>,
}
impl NativeResources {
    pub(super) fn prepare(role: ExecutionRole, server_thread: bool) -> super::Result<Self> {
        let owner = NativeRuntime::prepare(role, server_thread)?;
        Ok(Self {
            content: owner.runtime.content().clone(),
            prepared: Mutex::new(Some(owner)),
        })
    }
    pub(super) fn is_empty(&self) -> bool {
        self.content == ContentSnapshot::default()
    }
    pub(super) fn resource_root(&self, base: &Path) -> super::Result<PathBuf> {
        horizon_client::native_content::resource_root(
            base,
            &self.content,
            &std::env::current_dir()?.join(".horizon-cache/native"),
        )
    }
    pub(super) fn start(
        &self,
        role: ExecutionRole,
        server_thread: bool,
    ) -> super::Result<NativeRuntime> {
        let prepared = self
            .prepared
            .lock()
            .map_err(|_| "native preparation lock poisoned")?
            .take();
        let mut owner = match prepared {
            Some(owner) if owner.runtime.host_side() == host_side(role, server_thread) => owner,
            Some(_) | None => NativeRuntime::prepare(role, server_thread)?,
        };
        if owner.runtime.content() != &self.content {
            return Err("native registration differs from the frozen resource catalog".into());
        }
        owner.initialize()?;
        Ok(owner)
    }
}

impl Drop for NativeRuntime {
    fn drop(&mut self) {
        let report = self.runtime.shutdown();
        output(report.actions);
        for error in report.errors {
            log::error!("{error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_preparation_defers_setup_and_serves_each_execution_role() {
        let resources = NativeResources::prepare(ExecutionRole::Client, true).unwrap();
        assert_eq!(
            resources
                .prepared
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .runtime
                .state(),
            horizon_native_runtime::State::Frozen
        );
        let client = resources.start(ExecutionRole::Client, true).unwrap();
        assert!(resources.prepared.lock().unwrap().is_none());
        assert_eq!(
            client.runtime.state(),
            horizon_native_runtime::State::Running
        );
        let server = resources
            .start(ExecutionRole::IntegratedServer, true)
            .unwrap();
        assert_eq!(server.runtime.host_side(), HostSide::Server);
        assert_eq!(client.runtime.content(), server.runtime.content());
    }

    #[test]
    fn inconsistent_later_registration_cannot_open_a_world() {
        let mut resources =
            NativeResources::prepare(ExecutionRole::DedicatedServer, false).unwrap();
        resources
            .content
            .resources
            .insert("assets/unknown/mismatch".into(), vec![]);
        assert!(
            resources
                .start(ExecutionRole::DedicatedServer, false)
                .is_err()
        );
    }

    #[test]
    fn fixed_execution_roles_select_runtime_sides() {
        assert_eq!(host_side(ExecutionRole::Client, true), HostSide::Client);
        assert_eq!(host_side(ExecutionRole::Client, false), HostSide::Combined);
        for role in [
            ExecutionRole::IntegratedServer,
            ExecutionRole::DedicatedServer,
        ] {
            for threaded in [false, true] {
                assert_eq!(host_side(role, threaded), HostSide::Server);
            }
        }
    }

    #[test]
    fn owner_starts_the_compiled_pack_without_game_assets() {
        let owner = NativeRuntime::start(ExecutionRole::DedicatedServer, false).unwrap();
        assert_eq!(
            owner.runtime.state(),
            horizon_native_runtime::State::Running
        );
        drop(owner);
    }

    #[test]
    fn owner_cleans_up_on_early_return() {
        use horizon_native_api::{API_VERSION, ModContext, ModMetadata, ModSide, NativeMod};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        static META: ModMetadata = ModMetadata {
            id: "cleanup_test",
            name: "Cleanup",
            version: "1",
            description: "",
            authors: &[],
            license: "MIT",
            api_version: API_VERSION,
            side: ModSide::Common,
            dependencies: &[],
        };
        struct Fails(Arc<AtomicUsize>);
        impl NativeMod for Fails {
            fn metadata(&self) -> &'static ModMetadata {
                &META
            }
            fn initialize(&mut self, _: &mut ModContext) -> Result<(), String> {
                Err("setup failed".into())
            }
            fn shutdown(&mut self, _: &mut ModContext) -> Result<(), String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }
        fn start(
            count: Arc<AtomicUsize>,
        ) -> Result<NativeRuntime, horizon_native_runtime::LoaderError> {
            let mut owner = NativeRuntime {
                runtime: Runtime::new(vec![Box::new(Fails(count))])?,
                gameplay: Default::default(),
            };
            owner.runtime.initialize()?;
            Ok(owner)
        }
        let count = Arc::new(AtomicUsize::new(0));
        assert!(start(count.clone()).is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
