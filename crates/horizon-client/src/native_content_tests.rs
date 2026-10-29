use super::*;
use horizon_native_api::{CreativeTabDescriptor, ItemDescriptor};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "horizon-native-assets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let base = dir.join("base");
        for (path, value) in [
            ("horizon-resources.json", json!({"fixture":true})),
            ("facts.json", json!({"states":[],"defaults":{}})),
            ("gameplay-facts.json", json!({"states":[]})),
            (
                "items.json",
                json!({"minecraft:stick":{"minecraft:max_stack_size":64,"minecraft:item_model":"minecraft:stick","minecraft:item_name":{"translate":"item.minecraft.stick"}}}),
            ),
            (
                "registries.json",
                json!({"minecraft:item":{"entries":{"minecraft:stick":{"protocol_id":0}}},"minecraft:block":{"entries":{}}}),
            ),
            (
                "command-registries.json",
                json!({"items":["minecraft:stick"],"blocks":{}}),
            ),
            (
                "creative-inventory.json",
                json!({"tabs":[{"id":"minecraft:search","type":"SEARCH","column":0,"row":"TOP","items":[]}]}),
            ),
        ] {
            write(&base, path, &serde_json::to_vec(&value).unwrap()).unwrap();
        }
        fs::create_dir_all(base.join("data/minecraft/loot_table")).unwrap();
        Self(dir)
    }
    fn base(&self) -> PathBuf {
        self.0.join("base")
    }
    fn root(&self, c: &ContentSnapshot) -> Result<PathBuf> {
        resource_root(&self.base(), c, &self.0.join("cache"))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn pair(reverse: bool) -> ContentSnapshot {
    let mut snapshot = ContentSnapshot::default();
    let mut owners = vec!["alpha", "beta"];
    if reverse {
        owners.reverse();
    }
    for owner in owners {
        let id: ResourceId = format!("{owner}:gear").parse().unwrap();
        snapshot.items.insert(
            id.clone(),
            ItemDescriptor {
                id: id.clone(),
                template: "minecraft:stick".parse().unwrap(),
                translation_key: format!("label.{owner}"),
            },
        );
        let tab: ResourceId = format!("{owner}:tools").parse().unwrap();
        snapshot.tabs.insert(
            tab.clone(),
            CreativeTabDescriptor {
                id: tab,
                translation_key: format!("tab.{owner}"),
                icon: id.clone(),
                items: vec![id],
            },
        );
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[18, 140, 220, 255])
                .unwrap();
        }
        for (path, bytes) in [
            (format!("assets/{owner}/items/gear.json"), serde_json::to_vec(&json!({"model":{"type":"minecraft:model","model":format!("{owner}:item/gear")}})).unwrap()),
            (format!("assets/{owner}/models/item/gear.json"), serde_json::to_vec(&json!({"parent":"minecraft:builtin/generated","textures":{"layer0":format!("{owner}:item/gear")}})).unwrap()),
            (format!("assets/{owner}/textures/item/gear.png"), png),
            (format!("assets/{owner}/lang/en_us.json"), serde_json::to_vec(&json!({format!("label.{owner}"):format!("{owner} Gear"),format!("tab.{owner}"):format!("{owner} Tools")})).unwrap()),
            (format!("assets/{owner}/lang/ru_ru.json"), serde_json::to_vec(&json!({format!("label.{owner}"):"Шестерня",format!("tab.{owner}"):"Инструменты"})).unwrap()),
        ] { snapshot.resources.insert(path,bytes); }
    }
    // Each independent producer contributes exclusively through the public API.
    static ALPHA: horizon_native_api::ModMetadata = horizon_native_api::ModMetadata {
        id: "alpha",
        name: "Alpha",
        version: "1",
        description: "",
        authors: &[],
        license: "MIT",
        api_version: horizon_native_api::API_VERSION,
        side: horizon_native_api::ModSide::Common,
        dependencies: &[],
    };
    static BETA: horizon_native_api::ModMetadata = horizon_native_api::ModMetadata {
        id: "beta",
        name: "Beta",
        ..ALPHA
    };
    struct Producer(&'static horizon_native_api::ModMetadata, ContentSnapshot);
    impl horizon_native_api::NativeMod for Producer {
        fn metadata(&self) -> &'static horizon_native_api::ModMetadata {
            self.0
        }
        fn register(
            &mut self,
            c: &mut horizon_native_api::LoadContext,
        ) -> std::result::Result<(), String> {
            for item in self
                .1
                .items
                .values()
                .filter(|i| i.id.namespace() == self.0.id)
            {
                c.register_item(item.clone());
            }
            for tab in self
                .1
                .tabs
                .values()
                .filter(|t| t.id.namespace() == self.0.id)
            {
                c.register_tab(tab.clone());
            }
            for (path, bytes) in &self.1.resources {
                if path.split('/').nth(1) == Some(self.0.id) {
                    c.register_resource(path.clone(), bytes.clone());
                }
            }
            Ok(())
        }
    }
    let mut mods: Vec<Box<dyn horizon_native_api::NativeMod>> = vec![
        Box::new(Producer(&ALPHA, snapshot.clone())),
        Box::new(Producer(&BETA, snapshot)),
    ];
    if reverse {
        mods.reverse();
    }
    let mut runtime = horizon_native_runtime::Runtime::new(mods).unwrap();
    runtime.freeze().unwrap();
    runtime.content().clone()
}

#[test]
fn items_tabs_names_nbt_and_protocol_ids_use_existing_loaders() {
    let fixture = Fixture::new();
    let mut expected = None;
    for reverse in [false, true] {
        let root = fixture.root(&pair(reverse)).unwrap();
        let items = crate::items::Items::catalog(&root).unwrap();
        assert_eq!(items.get("alpha:gear").unwrap().max_stack, 64);
        assert_eq!(items.get("beta:gear").unwrap().model, "beta:gear");
        assert!(items.get("minecraft:stick").is_some());
        let c = Creative::load(&root).unwrap();
        assert_eq!(c.tabs.len(), 3);
        assert_eq!(c.tabs[1].items[0].item.as_ref(), "alpha:gear");
        let names = crate::creative::Names::load(&root);
        assert_eq!(names.item("alpha:gear"), "alpha Gear");
        let source = root.clone();
        let russian = crate::lang::Language::load(&[&source], "ru_ru");
        assert_eq!(russian.get("item.alpha.gear"), Some("Шестерня"));
        assert_eq!(russian.get("tab.alpha"), Some("Инструменты"));
        let stack = crate::items::ItemStack::new("alpha:gear", 7);
        let inventory = horizon_items::item_nbt::encode_items(&[Some(stack.clone())]);
        let decoded = horizon_items::item_nbt::decode_items(Some(&inventory), 1);
        assert_eq!(decoded[0].as_ref().unwrap().item, stack.item);
        assert_eq!(decoded[0].as_ref().unwrap().count, 7);
        let registry = json_file(&root.join("registries.json")).unwrap();
        let ids = registry["minecraft:item"]["entries"].clone();
        assert_eq!(ids["minecraft:stick"]["protocol_id"], 0);
        assert_eq!(ids["alpha:gear"]["protocol_id"], 1);
        assert_eq!(ids["beta:gear"]["protocol_id"], 2);
        if let Some(expected) = &expected {
            assert_eq!(&ids, expected);
        } else {
            expected = Some(ids);
        }
        // Loading the old inventory with the modded catalog retains vanilla IDs.
        let old = crate::items::ItemStack::new("minecraft:stick", 3);
        assert_eq!(
            horizon_items::item_nbt::item_from_nbt(&horizon_items::item_nbt::item_to_nbt(&old))
                .unwrap()
                .item,
            old.item
        );
        assert_eq!(
            json_file(&fixture.base().join("items.json"))
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            1
        );
    }
}

#[test]
fn missing_resources_templates_and_collisions_refuse_activation() {
    for case in [
        "template",
        "texture",
        "model",
        "translation",
        "collision",
        "namespace",
        "foreign translation",
    ] {
        let fixture = Fixture::new();
        let mut c = pair(false);
        match case {
            "template" => {
                c.items.values_mut().next().unwrap().template = "minecraft:absent".parse().unwrap()
            }
            "texture" => {
                c.resources.remove("assets/alpha/textures/item/gear.png");
            }
            "model" => {
                c.resources.remove("assets/alpha/models/item/gear.json");
            }
            "translation" => {
                c.resources
                    .insert("assets/alpha/lang/en_us.json".into(), b"{}".to_vec());
            }
            "collision" => {
                let mut items = json_file(&fixture.base().join("items.json")).unwrap();
                items["alpha:gear"] = json!({});
                write(
                    &fixture.base(),
                    "items.json",
                    &serde_json::to_vec(&items).unwrap(),
                )
                .unwrap();
            }
            "foreign translation" => {
                c.items.values_mut().next().unwrap().translation_key = "label.beta".into();
            }
            "namespace" => {
                write(&fixture.base(), "horizon-mod-content.json", b"[\"alpha\"]").unwrap()
            }
            _ => unreachable!(),
        }
        assert!(fixture.root(&c).is_err(), "{case}");
    }
}

#[test]
fn empty_native_pack_keeps_original_resource_root() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.root(&ContentSnapshot::default()).unwrap(),
        fixture.base()
    );
}

#[test]
fn static_pack_identity_ignores_cache_paths_and_registration_order() {
    let a = Fixture::new();
    let b = Fixture::new();
    let one = a.root(&pair(false)).unwrap();
    let two = b.root(&pair(true)).unwrap();
    let first = crate::registry_pack::load(&one).unwrap().unwrap();
    assert_eq!(
        Some(first.clone()),
        crate::registry_pack::load(&two).unwrap()
    );
    let mut changed = pair(false);
    changed
        .resources
        .insert("assets/alpha/lang/ru_ru.json".into(), b"{}".to_vec());
    let changed = b.root(&changed).unwrap();
    assert_ne!(Some(first), crate::registry_pack::load(&changed).unwrap());
}

#[test]
fn native_overlay_preserves_existing_content_namespaces_and_vanilla_inventory() {
    let fixture = Fixture::new();
    let mut items = json_file(&fixture.base().join("items.json")).unwrap();
    items["legacy:gem"] = json!({"minecraft:max_stack_size":16});
    write(
        &fixture.base(),
        "items.json",
        &serde_json::to_vec(&items).unwrap(),
    )
    .unwrap();
    let mut registries = json_file(&fixture.base().join("registries.json")).unwrap();
    registries["minecraft:item"]["entries"]["legacy:gem"] = json!({"protocol_id":1});
    write(
        &fixture.base(),
        "registries.json",
        &serde_json::to_vec(&registries).unwrap(),
    )
    .unwrap();
    write(&fixture.base(), "horizon-mod-content.json", b"[\"legacy\"]").unwrap();
    write(
        &fixture.base(),
        "assets/legacy/lang/en_us.json",
        br#"{"item.legacy.gem":"Legacy Gem"}"#,
    )
    .unwrap();
    let old = crate::items::ItemStack::new("minecraft:stick", 3);
    let old_inventory = horizon_items::item_nbt::encode_items(&[Some(old)]);
    let root = fixture.root(&pair(false)).unwrap();
    assert_eq!(mod_content::namespaces(&root), ["alpha", "beta", "legacy"]);
    let catalog = crate::items::Items::catalog(&root).unwrap();
    assert_eq!(catalog.get("legacy:gem").unwrap().max_stack, 16);
    assert_eq!(
        crate::creative::Names::load(&root).item("legacy:gem"),
        "Legacy Gem"
    );
    let decoded = horizon_items::item_nbt::decode_items(Some(&old_inventory), 1);
    assert_eq!(
        decoded[0].as_ref().unwrap().item.as_ref(),
        "minecraft:stick"
    );
    assert_eq!(decoded[0].as_ref().unwrap().count, 3);
    let registries = json_file(&root.join("registries.json")).unwrap();
    assert_eq!(
        registries["minecraft:item"]["entries"]["legacy:gem"]["protocol_id"],
        1
    );
    // The derived marker write must not follow the base marker's overlay link.
    assert_eq!(mod_content::namespaces(&fixture.base()), ["legacy"]);
}
