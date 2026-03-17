use crate::config::RawConfig;
use crate::tui::app::{AppState, ForceTreeState, RATING_OPTIONS, TreeNode};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

pub fn build_tree(config: &RawConfig) -> Vec<TreeNode> {
    let mut nodes = Vec::new();
    let servers = match &config.servers {
        Some(s) => s,
        None => return nodes,
    };

    for (server_label, server) in servers {
        nodes.push(TreeNode {
            label: server_label.clone(),
            depth: 0,
            is_library: false,
            server_label: server_label.clone(),
            library_label: None,
            location_label: None,
            force_rating: None,
        });

        if let Some(libraries) = &server.libraries {
            for (lib_label, lib) in libraries {
                nodes.push(TreeNode {
                    label: lib_label.clone(),
                    depth: 1,
                    is_library: true,
                    server_label: server_label.clone(),
                    library_label: Some(lib_label.clone()),
                    location_label: None,
                    force_rating: lib.force_rating.clone(),
                });

                if let Some(locations) = &lib.locations {
                    for (loc_label, loc) in locations {
                        nodes.push(TreeNode {
                            label: loc_label.clone(),
                            depth: 2,
                            is_library: false,
                            server_label: server_label.clone(),
                            library_label: Some(lib_label.clone()),
                            location_label: Some(loc_label.clone()),
                            force_rating: loc.force_rating.clone(),
                        });
                    }
                }
            }
        }
    }

    nodes
}

pub fn init_force_state(state: &mut AppState) {
    let nodes = build_tree(&state.config);
    let expanded: std::collections::HashSet<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.is_library)
        .map(|(i, _)| i)
        .collect();

    // Find first non-header node for initial cursor
    let initial_cursor = nodes.iter().position(|n| n.depth > 0).unwrap_or(0);

    state.force_state = ForceTreeState {
        nodes,
        cursor: initial_cursor,
        radio_cursor: 0,
        expanded,
    };

    // Set radio_cursor to match current rating of cursor node
    if let Some(node) = state.force_state.nodes.get(state.force_state.cursor) {
        state.force_state.radio_cursor = rating_to_index(&node.force_rating);
    }
}

pub fn rating_to_index(rating: &Option<String>) -> usize {
    match rating.as_deref() {
        None | Some("") => 0,
        Some("G") => 1,
        Some("PG-13") => 2,
        Some("R") => 3,
        _ => 0,
    }
}

fn rating_color(rating: Option<&str>) -> Color {
    match rating {
        Some("G") => Color::Green,
        Some("PG-13") => Color::Yellow,
        Some("R") => Color::Red,
        _ => Color::DarkGray,
    }
}

/// Check if a node at the given index is visible (not under a collapsed parent).
pub fn is_node_visible(state: &ForceTreeState, index: usize) -> bool {
    let node = &state.nodes[index];
    if node.depth < 2 {
        return true;
    }
    // Location node: check if parent library is expanded
    let parent_idx = (0..index).rev().find(|&j| state.nodes[j].is_library);
    match parent_idx {
        Some(pi) => state.expanded.contains(&pi),
        None => true,
    }
}

/// Apply force rating from radio_cursor to the node at cursor position.
pub fn apply_force_rating(state: &mut AppState) {
    let cursor = state.force_state.cursor;
    let node = &state.force_state.nodes[cursor];
    if node.depth == 0 {
        return; // server header — not editable
    }

    let new_rating = match state.force_state.radio_cursor {
        1 => Some("G".to_string()),
        2 => Some("PG-13".to_string()),
        3 => Some("R".to_string()),
        _ => None,
    };

    let server_label = node.server_label.clone();
    let lib_label = node.library_label.clone();
    let loc_label = node.location_label.clone();

    let mut applied = false;

    if let Some(servers) = state.config.servers.as_mut()
        && let Some(server) = servers.get_mut(&server_label)
        && let Some(ref lib_name) = lib_label
        && let Some(libraries) = server.libraries.as_mut()
        && let Some(lib) = libraries.get_mut(lib_name)
    {
        if let Some(ref loc_name) = loc_label {
            if let Some(locations) = lib.locations.as_mut()
                && let Some(loc) = locations.get_mut(loc_name)
            {
                loc.force_rating = new_rating.clone();
                applied = true;
            }
        } else {
            lib.force_rating = new_rating.clone();
            applied = true;
        }
    }

    if applied {
        state.force_state.nodes[cursor].force_rating = new_rating;
        state.mark_dirty();
    } else {
        state.error_message =
            Some("Could not apply force rating: config structure mismatch".to_string());
    }
}

pub fn render_force_tree(state: &AppState, area: Rect, buf: &mut Buffer) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue))
        .title(" Force Rating Overrides ");
    let inner = block.inner(area);
    block.render(area, buf);

    if state.force_state.nodes.is_empty() {
        Paragraph::new("  No servers with libraries configured.")
            .style(Style::default().fg(Color::DarkGray))
            .render(inner, buf);
        return;
    }

    let mut y = inner.y;

    for (i, node) in state.force_state.nodes.iter().enumerate() {
        if y >= inner.y + inner.height {
            break;
        }

        if !is_node_visible(&state.force_state, i) {
            continue;
        }

        let is_cursor = i == state.force_state.cursor;
        let indent = "  ".repeat(node.depth);

        // Background for cursor row
        if is_cursor {
            for x in inner.x..inner.x + inner.width {
                buf[(x, y)].set_style(Style::default().bg(Color::DarkGray));
            }
        }

        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::raw(indent));

        if node.depth == 0 {
            // Server header
            spans.push(Span::styled(
                &node.label,
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ));
            buf.set_line(inner.x, y, &Line::from(spans), inner.width);
            y += 1;
            continue;
        }

        if node.is_library {
            let arrow = if state.force_state.expanded.contains(&i) {
                "▾ "
            } else {
                "▸ "
            };
            spans.push(Span::styled(arrow, Style::default().fg(Color::Yellow)));
        } else {
            spans.push(Span::raw("  "));
        }

        let name_style = if is_cursor {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(&node.label, name_style));

        // Pad to align radio groups
        let used: usize = spans.iter().map(|s| s.content.len()).sum();
        let pad = 30usize.saturating_sub(used);
        spans.push(Span::raw(" ".repeat(pad)));

        // Radio group
        let current_idx = rating_to_index(&node.force_rating);
        let labels = ["None", "G", "PG-13", "R"];
        for (opt_i, label) in labels.iter().enumerate() {
            let is_active = opt_i == current_idx;
            let is_radio_cursor = is_cursor && opt_i == state.force_state.radio_cursor;

            let style = if is_radio_cursor {
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else if is_active {
                let color = rating_color(RATING_OPTIONS[opt_i]);
                Style::default().fg(color).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let bracket = if is_active {
                format!("[{label}]")
            } else {
                format!(" {label} ")
            };
            spans.push(Span::styled(bracket, style));
            spans.push(Span::raw(" "));
        }

        buf.set_line(inner.x, y, &Line::from(spans), inner.width);
        y += 1;
    }
}
