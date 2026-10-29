//! Thin authoritative adapter. All behavior remains in public-API mod crates.
use super::{Game, lan::LanHost};
use horizon_native_api::{
    GameplayAction, GameplayEvent, PlayerSnapshot, QueuedAction, WorldSnapshot,
};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct GameplayState {
    pub players: BTreeMap<u128, PlayerSnapshot>,
    pub events: Vec<GameplayEvent>,
    pub actions: Vec<QueuedAction>,
    pub failed: bool,
}

impl Game<'_> {
    fn native_authority(&self) -> bool {
        self.remote.is_none()
            && !self._native_api.gameplay.failed
            && self._native_api.runtime.host_side() != horizon_native_runtime::HostSide::Client
            && (!self._native_api.runtime.content().block_entities.is_empty()
                || self._native_api.runtime.has_gameplay_subscribers()
                || self._native_api.runtime.has_commands())
    }

    fn native_players(&self, host: Option<&LanHost>) -> Vec<PlayerSnapshot> {
        let mut players = Vec::new();
        if self.options.execution_role.is_client()
            && self.remote.is_none()
            && let Some(p) = &self.player
        {
            players.push(PlayerSnapshot {
                id: self.player_uuid(),
                name: super::chat::PLAYER_NAME.into(),
                dimension: self.dim.id().parse().expect("dimension"),
                position: [p.position.x, p.position.y, p.position.z],
                health: p.health.health(),
            });
        }
        for g in host
            .or(self.lan.as_ref())
            .into_iter()
            .flat_map(|h| &h.hub.guests)
            .filter(|g| g.placed)
        {
            players.push(PlayerSnapshot {
                id: g.uuid,
                name: g.name.clone(),
                dimension: g.dimension.parse().expect("dimension"),
                position: g.position,
                health: g.health,
            });
        }
        players.sort_by_key(|p| p.id);
        players
    }

    /// Captured after a successful player operation, including as_guest's identity.
    pub(super) fn native_api_block_event(
        &mut self,
        position: [i32; 3],
        state: horizon_world::registry::BlockStateId,
        placed: bool,
    ) {
        if !self.native_authority() {
            return;
        }
        self.native_api_entity_at(position);
        let Some(p) = &self.player else {
            return;
        };
        let Some(s) = self.pack.assets.registry.state(state) else {
            return;
        };
        let state = if s.properties.is_empty() {
            s.name.clone()
        } else {
            format!(
                "{}[{}]",
                s.name,
                s.properties
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let player = PlayerSnapshot {
            id: self.lan_acting_uuid.unwrap_or_else(|| self.player_uuid()),
            name: self
                .lan_acting_name
                .clone()
                .unwrap_or_else(|| super::chat::PLAYER_NAME.into()),
            dimension: self.dim.id().parse().expect("dimension"),
            position: [p.position.x, p.position.y, p.position.z],
            health: p.health.health(),
        };
        if self._native_api.gameplay.events.len() < 4096 {
            self._native_api
                .gameplay
                .events
                .push(GameplayEvent::BlockChanged {
                    player,
                    position,
                    state,
                    placed,
                });
        } else {
            log::error!("native gameplay event queue overflow");
            self._native_api.gameplay.failed = true;
        }
    }

    fn native_dispatch(&mut self, event: &GameplayEvent) {
        if self._native_api.gameplay.failed {
            return;
        }
        match self._native_api.runtime.dispatch(event) {
            Ok(actions) => {
                if self._native_api.gameplay.actions.len() + actions.len() > 4096 {
                    log::error!("native gameplay action queue overflow");
                    self._native_api.gameplay.actions.clear();
                    self._native_api.gameplay.failed = true;
                } else {
                    self._native_api.gameplay.actions.extend(actions);
                }
            }
            Err(error) => {
                log::error!("{error}");
                self._native_api.gameplay.actions.clear();
                self._native_api.gameplay.failed = true;
            }
        }
    }

    /// Once per authoritative dimension pass, after ordinary gameplay. No remote
    /// window dispatches this. Actions run after the complete snapshot batch.
    pub(super) fn tick_native_gameplay(&mut self, running: bool) {
        if !self.native_authority() {
            return;
        }
        let players = self.native_players(None);
        let before = std::mem::take(&mut self._native_api.gameplay.players);
        for p in before
            .values()
            .filter(|p| !players.iter().any(|q| p.id == q.id))
        {
            self.native_dispatch(&GameplayEvent::PlayerLeft(p.clone()));
        }
        for p in players.iter().filter(|p| !before.contains_key(&p.id)) {
            self.native_dispatch(&GameplayEvent::PlayerJoined(p.clone()));
        }
        self._native_api.gameplay.players = players.iter().map(|p| (p.id, p.clone())).collect();
        let events = std::mem::take(&mut self._native_api.gameplay.events);
        for event in events {
            if matches!(&event,GameplayEvent::BlockChanged{player,..} if player.dimension.as_str()!=self.dim.id())
            {
                self._native_api.gameplay.events.push(event);
            } else if !self._native_api.gameplay.failed {
                self.native_dispatch(&event);
            }
        }
        if running && !self._native_api.gameplay.failed {
            self.native_dispatch(&GameplayEvent::WorldTick {
                world: WorldSnapshot {
                    dimension: self.dim.id().parse().expect("dimension"),
                    tick: self.ticks,
                },
                players,
            });
        }
        if running && !self._native_api.gameplay.failed {
            self.native_entity_ticks();
        }
        self.apply_native_actions(None);
    }

    /// Initialize registered data on ordinary checked player/command/action placements.
    pub(super) fn native_api_entity_at(&mut self, position: [i32; 3]) {
        use horizon_client::containers::BlockEntity;
        let Some(state) = self.pack.assets.registry.state(self.world.block(
            position[0],
            position[1],
            position[2],
        )) else {
            return;
        };
        let Some(descriptor) = self
            ._native_api
            .runtime
            .content()
            .block_entities
            .values()
            .find(|e| e.block.as_str() == state.name)
        else {
            return;
        };
        let key = (position[0], position[1], position[2]);
        if !self.world.block_entities.contains_key(&key) {
            self.world.block_entities.insert(
                key,
                BlockEntity::Extension {
                    id: descriptor.id.to_string(),
                    version: descriptor.version,
                    data: descriptor.fields.clone(),
                },
            );
            self.world
                .modified
                .insert((position[0] >> 4, position[2] >> 4));
            if let Some(log) = &mut self.world.block_log {
                log.push(key);
            }
        }
    }

    fn native_entity_ticks(&mut self) {
        use horizon_client::containers::BlockEntity;
        let mut entities: Vec<_> = self
            .world
            .block_entities
            .iter()
            .filter_map(|(pos, e)| match e {
                BlockEntity::Extension { id, version, data } => {
                    Some((*pos, id.clone(), *version, data.clone()))
                }
                _ => None,
            })
            .collect();
        entities.sort_by_key(|e| e.0);
        for (pos, id, version, fields) in entities {
            let Ok(id) = id.parse::<horizon_native_api::ResourceId>() else {
                continue;
            };
            let Some(descriptor) = self._native_api.runtime.content().block_entities.get(&id)
            else {
                continue;
            };
            if version != descriptor.version || fields.keys().ne(descriptor.fields.keys()) {
                log::error!("native block entity schema mismatch at {pos:?}: {id}");
                self._native_api.gameplay.failed = true;
                return;
            }
            self.native_dispatch(&GameplayEvent::BlockEntityTick {
                world: WorldSnapshot {
                    dimension: self.dim.id().parse().expect("dimension"),
                    tick: self.ticks,
                },
                entity: horizon_native_api::BlockEntitySnapshot {
                    id,
                    position: [pos.0, pos.1, pos.2],
                    version,
                    fields,
                },
            });
        }
    }

    /// Namespaced zero-argument commands. Ownership is resolved by the runtime.
    pub(super) fn native_api_command(
        &mut self,
        mut host: Option<&mut LanHost>,
        conn: Option<u32>,
        line: &str,
    ) -> bool {
        if !self.native_authority() {
            return false;
        }
        let mut words = line.split_whitespace();
        let Some(name) = words.next() else {
            return false;
        };
        let Ok(id) = name.parse::<horizon_native_api::ResourceId>() else {
            return false;
        };
        if !self
            ._native_api
            .runtime
            .content()
            .commands
            .contains_key(&id)
        {
            return false;
        }
        let player_id = match conn {
            Some(conn) => host
                .as_deref()
                .and_then(|h| h.hub.guest(conn))
                .map(|g| g.uuid),
            None => Some(self.player_uuid()),
        };
        let Some(player) = self
            .native_players(host.as_deref())
            .into_iter()
            .find(|p| Some(p.id) == player_id)
        else {
            return true;
        };
        if words.next().is_some() {
            self.native_tell(host, player.id, format!("/{id} takes no arguments"));
            return true;
        }
        match self._native_api.runtime.command(&id, &player) {
            Some(Ok(actions)) => {
                if self._native_api.gameplay.actions.len() + actions.len() > 4096 {
                    self._native_api.gameplay.actions.clear();
                    self._native_api.gameplay.failed = true;
                    log::error!("native gameplay command action queue overflow");
                } else {
                    self._native_api.gameplay.actions.extend(actions);
                }
            }
            Some(Err(error)) => {
                log::error!("{error}");
                self._native_api.gameplay.actions.clear();
                self._native_api.gameplay.failed = true;
            }
            None => return false,
        }
        self.apply_native_actions(host.as_deref_mut());
        true
    }

    pub(super) fn native_tell(&mut self, host: Option<&mut LanHost>, id: u128, text: String) {
        use horizon_client::text::Text;
        if self.options.execution_role.is_client()
            && self.remote.is_none()
            && self.player.is_some()
            && id == self.player_uuid()
        {
            self.native_chat(&text);
        } else if let Some(host) = host.or(self.lan.as_mut()) {
            let conn = host
                .hub
                .guests
                .iter()
                .find(|g| g.placed && g.uuid == id)
                .map(|g| g.conn);
            if let Some(conn) = conn {
                host.hub.send_to(
                    conn,
                    &horizon_protocol::packets::play::Clientbound::SystemChat(
                        horizon_protocol::packets::play::SystemChat {
                            content: horizon_client::lan::data::component_nbt(
                                &Text::literal(text).component(),
                            ),
                            overlay: false,
                        },
                    ),
                );
            }
        }
    }

    pub(super) fn apply_native_actions(&mut self, mut host: Option<&mut LanHost>) {
        if self._native_api.gameplay.failed {
            return;
        }
        let actions = std::mem::take(&mut self._native_api.gameplay.actions);
        // Validate the complete current batch before the first mutation.
        let checked: Result<Vec<_>, String> = actions
            .iter()
            .map(|a| match &a.action {
                GameplayAction::SetBlockEntity { dimension, entity } => {
                    let descriptor = self
                        ._native_api
                        .runtime
                        .content()
                        .block_entities
                        .get(&entity.id)
                        .ok_or_else(|| {
                            format!("{}: unknown block entity {}", a.mod_id, entity.id)
                        })?;
                    if entity.id.namespace() != a.mod_id
                        || entity.version != descriptor.version
                        || entity.fields.keys().ne(descriptor.fields.keys())
                    {
                        return Err("block entity owner/schema mismatch".into());
                    }
                    if !matches!(
                        dimension.as_str(),
                        "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
                    ) {
                        return Err("unknown block entity dimension".into());
                    }
                    if dimension.as_str() == self.dim.id() {
                        let pos = entity.position;
                        let block = self
                            .pack
                            .assets
                            .registry
                            .state(self.world.block(pos[0], pos[1], pos[2]))
                            .map(|s| s.name.as_str());
                        if block != Some(descriptor.block.as_str())
                            || !self
                                .world
                                .block_entities
                                .get(&(pos[0], pos[1], pos[2]))
                                .is_some_and(|e| e.id() == entity.id.as_str())
                        {
                            return Err("block entity action targets missing/wrong block".into());
                        }
                    }
                    Ok(None)
                }
                GameplayAction::Tell { .. } => Ok(None),
                GameplayAction::SetBlock {
                    dimension,
                    position,
                    state,
                } => {
                    if !matches!(
                        dimension.as_str(),
                        "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
                    ) {
                        return Err(format!("{}: unknown dimension {dimension}", a.mod_id));
                    }
                    let state =
                        super::render_control::parse_state(self.pack.assets.registry, state)
                            .map_err(|e| format!("{}: {e}", a.mod_id))?;
                    if dimension.as_str() == self.dim.id()
                        && (!self
                            .world
                            .chunks
                            .contains_key(&(position[0] >> 4, position[2] >> 4))
                            || !(-64..320).contains(&position[1]))
                    {
                        return Err(format!("{}: write outside loaded world", a.mod_id));
                    }
                    Ok(Some(state))
                }
            })
            .collect();
        let states = match checked {
            Ok(states) => states,
            Err(e) => {
                log::error!("native gameplay action batch rejected: {e}");
                return;
            }
        };
        for (queued, state) in actions.into_iter().zip(states) {
            match &queued.action {
                GameplayAction::SetBlockEntity { dimension, entity }
                    if dimension.as_str() == self.dim.id() =>
                {
                    let key = (entity.position[0], entity.position[1], entity.position[2]);
                    let descriptor = &self._native_api.runtime.content().block_entities[&entity.id];
                    if self
                        .pack
                        .assets
                        .registry
                        .state(self.world.block(key.0, key.1, key.2))
                        .map(|s| s.name.as_str())
                        != Some(descriptor.block.as_str())
                        || !self
                            .world
                            .block_entities
                            .get(&key)
                            .is_some_and(|e| e.id() == entity.id.as_str())
                    {
                        log::warn!("native block entity action target changed during batch");
                        continue;
                    }
                    self.world.block_entities.insert(
                        key,
                        horizon_client::containers::BlockEntity::Extension {
                            id: entity.id.to_string(),
                            version: entity.version,
                            data: entity.fields.clone(),
                        },
                    );
                    self.world.modified.insert((key.0 >> 4, key.2 >> 4));
                    if let Some(log) = &mut self.world.block_log {
                        log.push(key);
                    }
                }
                GameplayAction::Tell { player, text } => self.native_tell(
                    host.as_deref_mut(),
                    *player,
                    format!("[native:{}] {text}", queued.mod_id),
                ),
                GameplayAction::SetBlock {
                    dimension,
                    position,
                    ..
                } if dimension.as_str() == self.dim.id() => {
                    if let Err(e) =
                        self.place_state(*position, state.expect("checked state"), false)
                    {
                        log::warn!(
                            "native gameplay action from {} failed: {e:?}",
                            queued.mod_id
                        );
                    }
                }
                _ => self._native_api.gameplay.actions.push(queued),
            }
        }
    }
}
