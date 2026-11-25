//! The keys around a binding's run: what the firing shortcut typed (so Type
//! text can erase it) and the cancel shortcut, which is registered only
//! while its binding runs (typing, or a Repeat/Toggle loop).

use super::{binding_of_hotkey, hotkey_id, with, App, Source};
use crate::actions::typing::Erase;
use crate::config::type_text::erase_count;
use crate::config::{Binding, MAX_SHORTCUTS};
use crate::i18n::tf;
use crate::shortcut::Shortcut;
use crate::ui;
use crate::win::{hotkey, typer};
use std::collections::HashSet;

/// Hotkey index of the cancel shortcut: the id slot after a binding's own
/// shortcuts (each binding has four ids, see `hotkey_id`).
pub const CANCEL_INDEX: usize = MAX_SHORTCUTS;
const _: () = assert!(CANCEL_INDEX < 4);

/// Binding and shortcut index behind a hotkey or matcher id.
pub fn split_id(id: i32) -> (u32, usize) {
    let binding = binding_of_hotkey(id);
    (binding, (id - hotkey_id(binding, 0)) as usize)
}

/// Bindings whose cancel shortcut must be registered right now.
fn wanted(bindings: &[Binding], typing: &HashSet<u32>, looping: impl Fn(u32) -> bool) -> HashSet<u32> {
    bindings
        .iter()
        .filter(|b| b.cancel_shortcut.is_some() && (typing.contains(&b.id) || looping(b.id)))
        .map(|b| b.id)
        .collect()
}

impl App {
    /// What a run may erase. Only a hotkey typed anything: the Run button
    /// and the tray erase nothing.
    pub(super) fn erase_for_run(&self, b: &Binding, source: Source) -> Erase {
        if source != Source::Hotkey {
            return Erase::default();
        }
        let count = self.fired.get(&b.id).map_or(0, |&i| erase_count(b, i));
        Erase { count, window: typer::foreground_id() }
    }

    /// Registers and releases cancel shortcuts to match what is running.
    /// Call after a run or a repeat loop starts or ends.
    pub(super) fn sync_cancel_keys(&mut self) {
        let want = wanted(&self.config.bindings, &self.typing, |id| self.repeats.contains_key(&id));
        let stale: Vec<u32> = self.cancel_keys.keys().filter(|id| !want.contains(id)).copied().collect();
        for id in stale {
            if self.cancel_keys.remove(&id) == Some(true) {
                hotkey::unregister(self.host, hotkey_id(id, CANCEL_INDEX));
            }
        }
        for id in want {
            if self.cancel_keys.contains_key(&id) {
                continue; // registered, or already reported as unavailable
            }
            let Some(b) = self.config.binding(id) else { continue };
            let Some(sc @ Shortcut::Standard { mods, vk }) = b.cancel_shortcut.clone() else { continue };
            let name = b.name.clone();
            let registered = hotkey::register(self.host, hotkey_id(id, CANCEL_INDEX), mods, vk);
            self.cancel_keys.insert(id, registered.is_ok());
            if let Err(e) = registered {
                let details = format!("{} {e}", sc.label());
                self.set_result(false, tf("status.cancel_unavailable", &[("name", &name), ("details", &details)]));
            }
        }
    }

    pub(super) fn release_cancel_keys(&mut self) {
        for (id, registered) in self.cancel_keys.drain() {
            if registered {
                hotkey::unregister(self.host, hotkey_id(id, CANCEL_INDEX));
            }
        }
    }
}

/// The cancel shortcut was pressed: stop the binding's run and repeat loop.
pub fn cancel_pressed(id: u32) {
    with(|a| {
        a.cancel_run(id);
        a.stop_repeat(id);
        if let Some(name) = a.config.binding(id).map(|b| b.name.clone()) {
            a.set_result(true, tf("status.cancelled", &[("name", &name)]));
        }
        a.sync_cancel_keys();
    });
    ui::main_window::refresh();
}

#[cfg(test)]
mod tests {
    use super::super::{EMERGENCY_IDS, PROFILE_SWITCH_ID};
    use super::*;
    use crate::config::{ALL_PROFILES, MAX_BINDING_ID};

    #[test]
    fn ids_carry_the_binding_and_the_shortcut() {
        for index in 0..=CANCEL_INDEX {
            assert_eq!(split_id(hotkey_id(42, index)), (42, index));
        }
        let highest = hotkey_id(MAX_BINDING_ID - 1, CANCEL_INDEX);
        assert!(highest < EMERGENCY_IDS[0], "below the reserved ids");
        assert!((highest as u32) < PROFILE_SWITCH_ID);
    }

    #[test]
    fn cancel_keys_are_wanted_only_while_running() {
        let binding = |id, cancel: bool| {
            let mut b = Binding::new(id, ALL_PROFILES);
            b.cancel_shortcut = cancel.then_some(Shortcut::Standard { mods: 0, vk: 0x7B });
            b
        };
        let all = [binding(1, true), binding(2, true), binding(3, false), binding(4, true)];
        let typing = HashSet::from([1, 3]);
        assert_eq!(wanted(&all, &typing, |id| id == 2), HashSet::from([1, 2]));
        assert!(wanted(&all, &HashSet::new(), |_| false).is_empty(), "idle");
    }
}
