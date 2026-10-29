//! Optional exact identity for statically composed content. Uses the protocol's
//! existing known-pack negotiation; it does not remap or synchronize item IDs.
use horizon_protocol::packets::configuration::KnownPack;
use std::path::Path;
pub const FILE: &str = "horizon-registry-pack.json";
const NAMESPACE: &str = "horizon";
const ID: &str = "static-content-v1";

pub fn identity(version: String) -> KnownPack {
    KnownPack {
        namespace: NAMESPACE.into(),
        id: ID.into(),
        version,
    }
}
fn ours(pack: &KnownPack) -> bool {
    pack.namespace == NAMESPACE && pack.id == ID
}
pub fn load(root: &Path) -> Result<Option<KnownPack>, String> {
    let path = root.join(FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let digest = value["sha256"]
        .as_str()
        .ok_or("static-content identity has no sha256")?;
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid static-content sha256".into());
    }
    Ok(Some(identity(digest.to_owned())))
}

/// Client: both peers must either omit static content or offer exactly the same
/// identity. Do not acknowledge vanilla core; full dynamic registry data follows.
pub fn select(local: Option<&KnownPack>, offered: &[KnownPack]) -> Result<Vec<KnownPack>, String> {
    let required: Vec<_> = offered.iter().filter(|p| ours(p)).collect();
    match (local, required.as_slice()) {
        (None, []) => Ok(Vec::new()),
        (Some(local), [server]) if local == *server => Ok(vec![local.clone()]),
        _ => Err(
            "Static content mismatch: client and server require the same compiled content pack"
                .into(),
        ),
    }
}
/// Server: a client without the required static pack cannot safely decode items.
pub fn confirm(required: Option<&KnownPack>, selected: &[KnownPack]) -> Result<(), String> {
    if required.is_some_and(|p| !selected.contains(p)) {
        Err(
            "Static content mismatch: rebuild the client with this server's compiled content pack"
                .into(),
        )
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn static_identity_is_required_on_both_sides_before_play() {
        let a = identity("a".repeat(64));
        let b = identity("b".repeat(64));
        assert_eq!(
            select(Some(&a), std::slice::from_ref(&a)).unwrap(),
            std::slice::from_ref(&a)
        );
        assert!(select(Some(&a), &[b]).is_err());
        assert!(select(Some(&a), &[]).is_err());
        assert!(select(None, std::slice::from_ref(&a)).is_err());
        assert!(select(None, &[]).unwrap().is_empty());
        assert!(confirm(Some(&a), &[]).is_err());
        assert!(confirm(Some(&a), std::slice::from_ref(&a)).is_ok());
        assert!(confirm(None, &[]).is_ok());
    }
}

/// Stable world requirements; protocol numbers and resource paths are excluded.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldContent {
    pub items: std::collections::BTreeSet<String>,
    pub blocks: std::collections::BTreeSet<String>,
    pub block_entities: std::collections::BTreeMap<String, u32>,
}
const WORLD_FILE: &str = "horizon-required-content.json";
fn requirements(resources: &Path) -> Result<WorldContent, String> {
    let marker = resources.join(FILE);
    if !marker.exists() {
        return Ok(WorldContent::default());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&marker).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    value.get("required").map_or_else(
        || Ok(WorldContent::default()),
        |v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()),
    )
}
/// Read-only check before migrations or decoding chunks, including feature-off builds.
pub fn check_world(resources: &Path, world: &Path) -> Result<(), String> {
    let path = world.join(WORLD_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    let saved: WorldContent =
        serde_json::from_slice(&bytes).map_err(|e| format!("world content requirements: {e}"))?;
    let current = requirements(resources)?;
    if !saved.items.is_subset(&current.items)
        || !saved.blocks.is_subset(&current.blocks)
        || saved
            .block_entities
            .iter()
            .any(|(id, version)| current.block_entities.get(id) != Some(version))
    {
        return Err("World content mismatch: rebuild with the world's required items, blocks and block entity schema versions before opening it".into());
    }
    Ok(())
}
/// Called only while holding the world's ordinary session lock. Additions may
/// change protocol order; only persistent names/schema versions are recorded.
pub fn record_world(resources: &Path, world: &Path) -> Result<(), String> {
    check_world(resources, world)?;
    let current = requirements(resources)?;
    if current == WorldContent::default() {
        return Ok(());
    }
    let bytes = serde_json::to_vec(&current).map_err(|e| e.to_string())?;
    let path = world.join(WORLD_FILE);
    if std::fs::read(&path).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    std::fs::create_dir_all(world).map_err(|e| e.to_string())?;
    let temporary = world.join(format!("{WORLD_FILE}.tmp"));
    std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(temporary, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod world_tests {
    use super::*;
    #[test]
    fn saved_names_accept_additions_and_refuse_missing_content_or_changed_schema() {
        let scratch = std::env::temp_dir().join(format!(
            "horizon-world-requirements-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let resources = scratch.join("resources");
        let world = scratch.join("world");
        std::fs::create_dir_all(&resources).unwrap();
        check_world(&resources, &world).unwrap();
        record_world(&resources, &world).unwrap();
        assert!(!world.join(WORLD_FILE).exists());
        let mut content = WorldContent::default();
        content.blocks.insert("a:counter".into());
        content.items.insert("a:counter".into());
        content.block_entities.insert("a:data".into(), 1);
        let write = |content: &WorldContent| {
            std::fs::write(
                resources.join(FILE),
                serde_json::to_vec(&serde_json::json!({"required":content})).unwrap(),
            )
            .unwrap()
        };
        write(&content);
        record_world(&resources, &world).unwrap();
        let saved = std::fs::read(world.join(WORLD_FILE)).unwrap();
        content.items.insert("b:new".into());
        write(&content);
        check_world(&resources, &world).unwrap();
        content.block_entities.insert("a:data".into(), 2);
        write(&content);
        assert!(record_world(&resources, &world).is_err());
        assert_eq!(std::fs::read(world.join(WORLD_FILE)).unwrap(), saved);
        std::fs::remove_file(resources.join(FILE)).unwrap();
        assert!(check_world(&resources, &world).is_err());
        std::fs::remove_dir_all(scratch).unwrap();
    }
}
