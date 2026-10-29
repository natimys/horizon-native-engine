//! Frozen native declarations -> existing Horizon content/resource pipeline.
//! This module contains no mod identities and owns no gameplay registry.
use crate::{
    Result,
    creative::Creative,
    mod_content::{self, ContentMod},
};
use horizon_mods::{Content, Manifest};
use horizon_native_api::{ContentSnapshot, ResourceId};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

fn asset(id: &str, kind: &str, extension: &str) -> Result<String> {
    let id: ResourceId = id.parse()?;
    Ok(format!(
        "assets/{}/{kind}/{}.{extension}",
        id.namespace(),
        id.as_str().split_once(':').expect("validated ID").1
    ))
}
fn json_file(path: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(
        &fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?,
    )?)
}
fn write(root: &Path, path: &str, bytes: &[u8]) -> Result<()> {
    let path = root.join(path);
    fs::create_dir_all(path.parent().ok_or("resource without parent")?)?;
    if path.is_symlink() {
        fs::remove_file(&path)?;
    }
    fs::write(path, bytes)?;
    Ok(())
}

/// Fail before worlds open, rather than silently giving the missing-model icon.
/// Initial adapter supports ordinary static model items; dynamic/special item
/// models need a separately validated public capability.
fn validate(root: &Path, content: &ContentSnapshot) -> Result<()> {
    let items = json_file(&root.join("items.json"))?;
    for item in content.items.values() {
        let id = item.id.as_str();
        if items.get(id).is_some() {
            return Err(format!("item ID collision: {id}").into());
        }
        if item.template.namespace() != "minecraft" || items.get(item.template.as_str()).is_none() {
            return Err(format!("{id}: missing vanilla template {}", item.template).into());
        }
    }
    Ok(())
}

fn validate_models(root: &Path, content: &ContentSnapshot) -> Result<()> {
    use horizon_assets::{
        item_model::{ClientItem, ItemModelNode},
        model::CuboidModel,
        resolve::{GENERATED_MODEL, ModelLibrary},
    };
    for item in content.items.values() {
        let path = asset(item.id.as_str(), "items", "json")?;
        let text = fs::read_to_string(root.join(&path)).map_err(|e| format!("{path}: {e}"))?;
        let definition = ClientItem::from_json(&text)?;
        let ItemModelNode::Model(leaf) = definition.model else {
            return Err(format!("{}: native adapter requires a static model item", item.id).into());
        };
        let mut pending = vec![leaf.model.clone()];
        let mut models = BTreeMap::new();
        while let Some(name) = pending.pop() {
            if name == GENERATED_MODEL || models.contains_key(&name) {
                continue;
            }
            let path = asset(&name, "models", "json")?;
            let model = CuboidModel::from_json(
                &fs::read_to_string(root.join(&path)).map_err(|e| format!("{path}: {e}"))?,
            )?;
            if let Some(parent) = &model.parent {
                pending.push(parent.clone());
            }
            models.insert(name, model);
        }
        let resolved = ModelLibrary::with_item_generator(models)?.resolve(&leaf.model)?;
        if !resolved.unresolved_textures.is_empty() || resolved.textures.is_empty() {
            return Err(format!("{}: missing or unresolved item textures", item.id).into());
        }
        for material in resolved.textures.values() {
            let path = asset(&material.sprite, "textures", "png")?;
            let decoder = png::Decoder::new(std::io::BufReader::new(
                fs::File::open(root.join(&path)).map_err(|e| format!("{path}: {e}"))?,
            ));
            let mut reader = decoder.read_info()?;
            let mut pixels = vec![0; reader.output_buffer_size().ok_or("texture too large")?];
            reader.next_frame(&mut pixels)?;
        }
    }
    Ok(())
}

/// Reuse Content::read and mod_content::resource_root, including their registry
/// table adapters, namespaces, resource discovery and pack reload behavior.
/// All generated files go under the caller's game cache, never the base root.
pub fn resource_root(base: &Path, content: &ContentSnapshot, cache: &Path) -> Result<PathBuf> {
    if content == &ContentSnapshot::default() {
        return Ok(base.to_owned());
    }
    validate(base, content)?;
    let mut files = content.resources.clone();
    for item in content.items.values() {
        let path = format!(
            "data/{}/horizon/item/{}.json",
            item.id.namespace(),
            item.id.as_str().split_once(':').expect("validated ID").1
        );
        if files
            .insert(
                path,
                serde_json::to_vec(&json!({"template": item.template.as_str()}))?,
            )
            .is_some()
        {
            return Err("native resource attempts to redefine a declaration".into());
        }
    }
    let mut hash = Sha256::new();
    for (path, bytes) in &files {
        hash.update((path.len() as u64).to_le_bytes());
        hash.update(path);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    // Tab/title changes also invalidate the derived overlay.
    hash.update(format!("{:?}", content.tabs));
    hash.update(format!("{:?}", content.items));
    let mut identity = hash.clone();
    hash.update(std::path::absolute(base)?.to_string_lossy().as_bytes());
    let cache = cache.join(format!("{:x}", hash.finalize()));
    let source = cache.join("sources");
    let mut owners = BTreeSet::new();
    for (path, bytes) in &files {
        horizon_mods::content::check_path(path)?;
        let owner = path.split('/').nth(1).ok_or("missing namespace")?;
        if owner == "minecraft" {
            return Err("native resources cannot replace minecraft".into());
        }
        owners.insert(owner.to_owned());
        let root = source.join(owner);
        if fs::read(root.join(path)).is_ok_and(|existing| existing == *bytes) {
            continue;
        }
        write(&root, path, bytes)?;
    }
    let mut mods = Vec::new();
    for owner in owners {
        // Native IDs were validated by the native runtime. Do not impose the
        // unrelated WASM package metadata/version policy on linked Rust crates.
        let manifest = Manifest {
            id: owner.clone(),
            name: owner.clone(),
            version: "1".into(),
            description: String::new(),
            authors: Vec::new(),
            api: "0.2".into(),
            side: horizon_mods::manifest::Side::Both,
            capabilities: vec![horizon_mods::Capability::Registries],
            namespaces: Vec::new(),
            experimental: Vec::new(),
        };
        mods.push(ContentMod {
            id: owner.clone(),
            content: Content::read(&source.join(owner), &manifest)?,
        });
    }
    // Native and WASM content must not share namespaces. The existing overlay
    // marker exposes all previously contributed namespaces.
    for m in &mods {
        if mod_content::namespaces(base).contains(&m.id) {
            return Err(format!("native/content namespace collision: {}", m.id).into());
        }
    }
    if mods.is_empty() {
        return Err("creative declarations require registered content".into());
    }
    let root = mod_content::resource_root(base, &mods, &cache)?;
    identity.update(serde_json::to_vec(&json_file(
        &root.join("registries.json"),
    )?)?);
    write(
        &root,
        crate::registry_pack::FILE,
        &serde_json::to_vec(&json!({"sha256":format!("{:x}",identity.finalize())}))?,
    )?;
    // Preserve earlier overlay namespaces so language and model discovery sees both.
    let mut namespaces: BTreeSet<_> = mod_content::namespaces(base).into_iter().collect();
    namespaces.extend(mod_content::namespaces(&root));
    write(
        &root,
        "horizon-mod-content.json",
        &serde_json::to_vec(&namespaces)?,
    )?;
    let mut english = BTreeMap::<String, String>::new();
    for ns in &namespaces {
        let path = root.join(format!("assets/{ns}/lang/en_us.json"));
        if path.exists() {
            english.extend(serde_json::from_slice::<BTreeMap<String, String>>(
                &fs::read(path)?,
            )?);
        }
    }
    // Names uses canonical item keys. Alias a descriptor's arbitrary key in each
    // supplied language, using the existing language readers afterwards.
    for (path, bytes) in &content.resources {
        if !path.contains("/lang/") || !path.ends_with(".json") {
            continue;
        }
        let mut strings: BTreeMap<String, String> = serde_json::from_slice(bytes)?;
        let owner = path.split('/').nth(1).ok_or("missing namespace")?;
        for item in content.items.values().filter(|i| i.id.namespace() == owner) {
            if let Some(name) = strings.get(&item.translation_key).cloned() {
                strings.insert(
                    format!(
                        "item.{}.{}",
                        owner,
                        item.id.as_str().split_once(':').expect("validated ID").1
                    ),
                    name,
                );
            }
        }
        write(&root, path, &serde_json::to_vec(&strings)?)?;
    }
    for key in content
        .items
        .values()
        .map(|i| &i.translation_key)
        .chain(content.tabs.values().map(|t| &t.translation_key))
    {
        if !english.contains_key(key) {
            return Err(format!("missing English translation: {key}").into());
        }
    }
    validate_models(&root, content)?;
    let stack = |id: &ResourceId| format!("{{count:1,id:{:?}}}", id.as_str());
    let mut tabs: Vec<_> = content.tabs.values().map(|t| json!({"id":t.id.as_str(),"title":t.translation_key,"type":"CATEGORY","icon":stack(&t.icon),"items":t.items.iter().map(&stack).collect::<Vec<_>>()})).collect();
    // Validate positions/collisions with the existing adapter before publishing.
    let mut creative = Creative::load(base)?;
    creative.append_tabs(Creative::from_json(&json!({"tabs":tabs}))?.tabs)?;
    let previous = base.join("creative-additions.json");
    if previous.exists() {
        let previous = json_file(&previous)?;
        tabs.extend(
            previous["tabs"]
                .as_array()
                .ok_or("invalid base creative additions")?
                .iter()
                .cloned(),
        );
    }
    write(
        &root,
        "creative-additions.json",
        &serde_json::to_vec(&json!({"tabs":tabs}))?,
    )?;
    Ok(root)
}

#[cfg(test)]
#[path = "native_content_tests.rs"]
mod tests;
