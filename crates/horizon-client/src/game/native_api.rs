//! Generic ownership bridge for the public-API native path. Independent of WASM/mixins.
use super::execution_role::ExecutionRole;
use horizon_native_api::ModAction;
use horizon_native_runtime::{HostSide, Runtime};

/// Drop also runs on startup/loop errors. Mods never receive Game or engine handles.
pub(super) struct NativeRuntime {
    runtime: Runtime,
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

    pub(super) fn start(role: ExecutionRole, server_thread: bool) -> super::Result<Self> {
        let runtime = horizon_native_pack::create_for_side(host_side(role, server_thread))?;
        let mut owner = Self { runtime };
        output(owner.runtime.initialize()?);
        for metadata in owner.runtime.metadata() {
            log::info!(
                "Native mod: {} {} ({})",
                metadata.name,
                metadata.version,
                metadata.id
            );
        }
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
            };
            owner.runtime.initialize()?;
            Ok(owner)
        }
        let count = Arc::new(AtomicUsize::new(0));
        assert!(start(count.clone()).is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
