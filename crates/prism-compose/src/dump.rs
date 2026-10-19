//! The text dump of a composition (D14 of `compose-runtime`): its group tree, one line per
//! group and per slot, indented by two spaces per level, then its applier's own dump. It holds
//! keys, kinds, tags, values and node numbers, never addresses or generations, so it is stable
//! enough for fixtures to compare.

use crate::composer::{ComposeResult, STALE_COMPOSITION};
use crate::composition::Composition;
use crate::group::GroupKind;
use crate::handle::Handle;
use crate::slot::Slot;
use crate::state::States;
use crate::thread::with_state;
use std::fmt::Write;

impl Composition {
    /// The dump of the composition.
    pub fn dump(composition: Handle) -> ComposeResult<String> {
        with_state(|state| {
            let record = match state.compositions.get(composition) {
                Some(record) if !record.disposed => record,
                _ => return Err(STALE_COMPOSITION),
            };
            let mut out = String::new();
            // Depth-first without recursion: (group, depth).
            let mut stack = vec![(record.root, 0usize)];
            while let Some((group, depth)) = stack.pop() {
                let record_group = state.groups.get(group).unwrap();
                let indent = "  ".repeat(depth);
                let invalid = if record_group.invalid { " invalid" } else { "" };
                if record_group.parent.is_none() {
                    let _ = writeln!(out, "root{}", invalid);
                } else {
                    let key = record_group.key;
                    let _ = match record_group.kind {
                        GroupKind::Restart => writeln!(out, "{}restart {}{}", indent, key, invalid),
                        GroupKind::Replaceable => writeln!(out, "{}replaceable {}", indent, key),
                        GroupKind::Movable => {
                            writeln!(out, "{}movable {} ({})", indent, key, record_group.data_key)
                        }
                        GroupKind::Node => writeln!(
                            out,
                            "{}node {} #{} kind {}",
                            indent,
                            key,
                            record_group
                                .node
                                .map_or(0, |node| record.applier.number(node)),
                            record_group.node_kind
                        ),
                    };
                }
                let inner = "  ".repeat(depth + 1);
                for slot in &record_group.slots {
                    let _ = match slot {
                        Slot::Word { tag, value } => {
                            writeln!(out, "{}word {} = {}", inner, tag, value)
                        }
                        // SAFETY: a slot's record is valid while the slot lives.
                        Slot::Typed {
                            slot_type, cells, ..
                        } => {
                            let line =
                                writeln!(out, "{}slot {}", inner, unsafe { (**slot_type).name() });
                            // Its state cells, one level deeper, in creation order (D20 of
                            // `state-handles`).
                            for cell in cells {
                                if let Some(name) = States::type_name(state, *cell) {
                                    let _ = writeln!(out, "{}  cell {}", inner, name);
                                }
                            }
                            line
                        }
                    };
                }
                for child in record_group.children.iter().rev() {
                    stack.push((*child, depth + 1));
                }
            }
            record.applier.dump(&mut out);
            Ok(out)
        })
    }
}
