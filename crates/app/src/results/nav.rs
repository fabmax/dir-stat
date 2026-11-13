//! The path bar: history buttons, breadcrumbs and the zoom root's total (UX §6.5).

use dirstat_core::tree::{NodeId, Tree};
use iced::widget::{Row, button, container, mouse_area, row, space, tooltip};
use iced::{Center, Element, Fill, mouse};

use super::Results;
use crate::app::Message;
use crate::keys::KeyAction;
use crate::{format, theme, widgets};

/// The zoom root's ancestors from the scan root down, with their display names.
pub fn crumbs(tree: &Tree, zoom: NodeId) -> Vec<(NodeId, String)> {
    let mut chain: Vec<NodeId> = tree.ancestors(zoom).collect();
    chain.reverse();
    chain.push(zoom);
    chain
        .into_iter()
        .map(|id| (id, tree.node(id).name().to_string_lossy().into_owned()))
        .collect()
}

/// Which crumbs fit in `width`: all, or the first, "…", and the last two.
pub fn visible_crumbs(names: &[String], width: f32, size: f32) -> (Vec<usize>, bool) {
    let all: f32 = names
        .iter()
        .map(|n| widgets::estimated_width(n, size) + 24.0)
        .sum();
    if all <= width || names.len() <= 3 {
        return ((0..names.len()).collect(), false);
    }
    (vec![0, names.len() - 2, names.len() - 1], true)
}

pub fn view<'a>(
    results: &'a Results,
    width: f32,
    home: Option<&std::path::Path>,
) -> Element<'a, Message> {
    let tree = &results.tree;
    let history = &results.view.history;
    let arrow = |label: &'static str, enabled: bool, action: KeyAction| {
        let b = button(theme::BODY.text(label))
            .padding([2, 8])
            .style(theme::secondary_button);
        if enabled {
            b.on_press(Message::Key(action))
        } else {
            b
        }
    };
    let mut bar = row![
        arrow("‹", history.can_go_back(), KeyAction::Back),
        arrow("›", history.can_go_forward(), KeyAction::Forward),
        space().width(8),
    ]
    .spacing(4)
    .align_y(Center);

    if !tree.is_started() {
        let path = tree.root_path().display().to_string();
        return container(
            bar.push(theme::BODY.text(path).width(Fill))
                .push(theme::BODY_STRONG.text(format::PENDING)),
        )
        .padding([0, 12])
        .height(32)
        .center_y(32)
        .into();
    }

    let zoom = results.view.zoom;
    let chain = crumbs(tree, zoom);
    let names: Vec<String> = chain.iter().map(|(_, n)| n.clone()).collect();
    let (shown, collapsed) = visible_crumbs(&names, width - 420.0, theme::BODY.size);
    let root_path = tree.root_path();
    let first_icon = if results.estimate.is_some() {
        "▭"
    } else if home == Some(root_path) {
        "⌂"
    } else {
        "▭"
    };
    let mut crumbs_row: Row<'a, Message> =
        row![theme::BODY.text(first_icon).style(theme::text_secondary)]
            .spacing(6)
            .align_y(Center);
    for (position, &index) in shown.iter().enumerate() {
        if position > 0 {
            crumbs_row = crumbs_row.push(theme::BODY.text("›").style(theme::text_muted));
        }
        if collapsed && position == 1 {
            let full = tree.path_of(zoom).display().to_string();
            crumbs_row = crumbs_row
                .push(tooltip(
                    theme::BODY.text("…").style(theme::text_secondary),
                    tip(full),
                    tooltip::Position::Bottom,
                ))
                .push(theme::BODY.text("›").style(theme::text_muted));
        }
        let (id, name) = &chain[index];
        let current = *id == zoom;
        let label = theme::BODY
            .with_font(if current {
                crate::fonts::MEDIUM
            } else {
                crate::fonts::INTER
            })
            .text(name.clone());
        let crumb = mouse_area(label)
            .on_press(Message::ZoomTo(*id))
            .interaction(mouse::Interaction::Pointer);
        crumbs_row = crumbs_row.push(crumb);
    }
    bar = bar.push(container(crumbs_row).width(Fill).clip(true));

    let total = tree.node(zoom).total().alloc;
    let size = if tree.is_lower_bound(zoom) || results.is_scanning() {
        format::size_at_least(total)
    } else {
        format::size(total)
    };
    let mut total_row = row![theme::BODY_STRONG.text(size)]
        .spacing(4)
        .align_y(Center);
    if zoom != NodeId::ROOT {
        let root_total = tree.node(NodeId::ROOT).total().alloc;
        let share = format!(
            "· {} of {}",
            format::percent(total, root_total),
            results.title
        );
        total_row = total_row.push(theme::SMALL.text(share).style(theme::text_secondary));
    }
    container(bar.push(total_row))
        .padding([0, 12])
        .height(32)
        .center_y(32)
        .into()
}

fn tip<'a>(text: String) -> Element<'a, Message> {
    container(theme::SMALL.text(text))
        .padding([4, 8])
        .max_width(480)
        .style(theme::raised)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dirstat_core::tree::testing::{TreeBuilder, dir};

    #[test]
    fn crumbs_run_from_the_root_and_collapse() {
        let tree = TreeBuilder::new(
            "/r",
            dir("r").child(dir("a").child(dir("b").child(dir("c")))),
        )
        .build();
        let names: Vec<String> = crumbs(&tree, NodeId(3))
            .into_iter()
            .map(|(_, n)| n)
            .collect();
        assert_eq!(names, ["r", "a", "b", "c"]);
        assert_eq!(
            visible_crumbs(&names, 1000.0, 13.0),
            (vec![0, 1, 2, 3], false)
        );
        assert_eq!(
            visible_crumbs(&names, 50.0, 13.0),
            (vec![0, 2, 3], true),
            "first + … + last two"
        );
        assert!(
            !visible_crumbs(&names[..3], 10.0, 13.0).1,
            "three always fit"
        );
    }
}
