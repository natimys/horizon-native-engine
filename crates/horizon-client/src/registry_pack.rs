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
            [a.clone()]
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
