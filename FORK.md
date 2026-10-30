# Horizon Native Mod Loader engine fork

This is the engine integration fork for
[Horizon Native Mod Loader](https://github.com/natimys/horizon-native-mod-loader),
an independent experimental project. Upstream Horizon is
[horizon-rust/horizon](https://github.com/horizon-rust/horizon). Upstream's README,
MIT copyright notice, contribution rules, original WASM loader and history are
preserved. Upstream download links and badges describe upstream, not this fork.

## Build through the outer repository

Clone the outer project with its pinned `horizon/` submodule. Native API, runtime,
UI and static pack crates live in the sibling `native-mods/` workspace, and the
engine's optional path dependencies reference them. This fork alone is not a
complete native build. Follow the outer README and exact toolchain/source lock.
Default builds keep native hooks disabled; the existing WASM loader is separate.

## Patch provenance and review

The integration descends from upstream commit
`cad2a29d2ad60a9fa001f8e1a087becf13665519`. The initial 22 integration commits
end at `f84396c94ebc52f9abb072522b8779a8834362b7`; subsequent preparation commits
add publication documentation and isolate inherited automation. Review engine
changes with `git diff cad2a29d2ad60a9fa001f8e1a087becf13665519..HEAD -- crates Cargo.toml Cargo.lock`.
The outer `docs/native-modding/HORIZON_INTEGRATION_MAP.md` records hook entry
points, rationale, alternatives and validation. Concrete mod behavior belongs
in mod crates and depends only on the public API.

The fork retains Horizon's MIT license (Ludovic Marie and the Horizon
contributors), third-party notices and component licenses. Original downstream
changes are MIT; copyright (c) 2026 natimys and the Horizon Native Mod Loader
contributors. No Minecraft resources, personal saves or compiled releases are
provided by this fork.

## Automation and upstream maintenance

Upstream workflows are preserved byte-for-byte in `.github/upstream-workflows/`
instead of the executable `.github/workflows/` directory. They include release,
deployment, signing, registry and private-runner jobs intended for upstream's
infrastructure. Do not enable or restore them wholesale in this fork. The outer
project runs its own read-only, asset-free CI against the exact fork commit.

Keep `upstream` pointing at horizon-rust/horizon and `origin` at
natimys/horizon-native-engine. Develop on `codex/native-integration`. Fetch and
review upstream changes before an explicit merge; do not rebase/squash the
published integration history. Do not mirror-push upstream refs, tags or
unrelated branches. An upstream update also needs an outer submodule and source
lock update plus vanilla/native/WASM regression validation.

Report integration bugs to the outer project's issue tracker. For a change
proposed upstream, follow the original CONTRIBUTING.md and AGENTS.md, retain
attribution, and provide the relevant parity evidence.
