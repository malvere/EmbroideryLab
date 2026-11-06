//! Preflight local SVG clones before usvg expands them. Its parser otherwise
//! skips unresolved/cyclic references; an embroidery import must not silently
//! lose a visible shape. Limits apply even to definitions, which usvg visits.
use std::collections::BTreeMap;

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 128;

#[derive(Clone, Copy, Default)]
struct Expansion {
    nodes: usize,
    height: usize,
}

fn expansion(
    index: usize,
    depth: usize,
    edges: &[Vec<usize>],
    state: &mut [u8],
    memo: &mut [Expansion],
) -> Result<Expansion, String> {
    if depth > MAX_DEPTH {
        return Err("SVG reference expansion exceeds 128 nested nodes".into());
    }
    if state[index] == 1 {
        return Err("SVG contains cyclic local use references".into());
    }
    if state[index] == 2 {
        if depth.saturating_add(memo[index].height) > MAX_DEPTH + 1 {
            return Err("SVG reference expansion exceeds 128 nested nodes".into());
        }
        return Ok(memo[index]);
    }
    state[index] = 1;
    let mut total = Expansion {
        nodes: 1,
        height: 1,
    };
    for &child in &edges[index] {
        let child = expansion(child, depth + 1, edges, state, memo)?;
        total.nodes = total.nodes.saturating_add(child.nodes);
        total.height = total.height.max(child.height + 1);
        if total.nodes > MAX_NODES {
            return Err("SVG reference expansion exceeds 100,000 nodes".into());
        }
    }
    state[index] = 2;
    memo[index] = total;
    Ok(total)
}

/// Validate clone targets without fetching external resources or guessing
/// which duplicate ID the artwork author intended. Geometry stays with usvg.
pub(super) fn validate(document: &roxmltree::Document<'_>) -> Result<(), String> {
    let nodes: Vec<_> = document.root().descendants().take(MAX_NODES + 1).collect();
    if nodes.len() > MAX_NODES {
        return Err("SVG reference checks exceed 100,000 XML nodes".into());
    }
    let indices: BTreeMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id().get_usize(), index))
        .collect();
    let mut ids = BTreeMap::new();
    for (index, node) in nodes.iter().enumerate().filter(|(_, n)| n.is_element()) {
        let mut attributes = node.attributes().filter(|a| a.name() == "id");
        if let Some(attribute) = attributes.next() {
            if attributes.next().is_some() {
                return Err("SVG contains ambiguous ID attributes".into());
            }
            let id = attribute.value();
            if !id.is_empty() && ids.insert(id, index).is_some() {
                return Err(format!("SVG contains duplicate ID '{id}'"));
            }
        }
    }
    let mut edges = vec![Vec::new(); nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        let is_use = node.is_element()
            && node.tag_name().namespace() == Some(SVG_NS)
            && node.tag_name().name() == "use";
        if !is_use {
            edges[index].extend(
                node.children()
                    .map(|child| indices[&child.id().get_usize()]),
            );
            continue;
        }
        // Match SVG2/usvg precedence even when xlink:href appears first.
        let href = node
            .attributes()
            .find(|a| a.name() == "href" && a.namespace().is_none())
            .or_else(|| {
                node.attributes()
                    .find(|a| a.name() == "href" && a.namespace() == Some(XLINK_NS))
            })
            .map(|a| a.value().trim())
            .ok_or("SVG use requires a local #id reference")?;
        let Some(id) = href.strip_prefix('#') else {
            return Err("SVG external use references are unsupported; use a local #id".into());
        };
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            return Err("SVG use requires a nonempty local #id reference".into());
        }
        let &target = ids
            .get(id)
            .ok_or_else(|| format!("SVG use references missing ID '{id}'"))?;
        let referenced = nodes[target];
        if referenced.tag_name().namespace() != Some(SVG_NS)
            || !matches!(
                referenced.tag_name().name(),
                "svg"
                    | "g"
                    | "a"
                    | "switch"
                    | "symbol"
                    | "use"
                    | "path"
                    | "rect"
                    | "circle"
                    | "ellipse"
                    | "line"
                    | "polyline"
                    | "polygon"
            )
        {
            return Err(format!(
                "SVG use target '{id}' is not a supported vector element"
            ));
        }
        edges[index].push(target);
    }
    let mut state = vec![0; nodes.len()];
    let mut memo = vec![Expansion::default(); nodes.len()];
    // Also inspect any children ignored by a malformed use element, rather
    // than letting their invalid references depend on a parser's skip policy.
    for index in 0..nodes.len() {
        expansion(index, 1, &edges, &mut state, &mut memo)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(body: &str) -> Result<(), String> {
        let xml = format!(
            "<svg xmlns='{SVG_NS}' xmlns:xlink='{XLINK_NS}' width='100' height='100'>{body}</svg>"
        );
        validate(&roxmltree::Document::parse(&xml).unwrap())
    }

    #[test]
    fn local_groups_symbols_and_nested_clones_are_bounded_without_false_cycles() {
        check(
            "<defs><g id='shape'><path d='M0 0H10V10Z'/></g><symbol id='symbol'><use href='#shape'/></symbol></defs><use xlink:href='#symbol'/><use href='#shape'/>",
        )
        .unwrap();
    }

    #[test]
    fn unprefixed_href_overrides_xlink_independent_of_attribute_order() {
        for attributes in [
            "href='#shape' xlink:href='outside.svg#shape'",
            "xlink:href='outside.svg#shape' href='#shape'",
        ] {
            check(&format!(
                "<defs><path id='shape' d='M0 0H10V10Z'/></defs><use {attributes}/>"
            ))
            .unwrap();
        }
        assert!(
            check("<path id='shape'/><use xlink:href='#shape' href='outside.svg#shape'/>")
                .unwrap_err()
                .contains("external")
        );
    }

    #[test]
    fn invalid_missing_external_and_ambiguous_references_are_explicit() {
        for (body, expected) in [
            ("<use/>", "requires a local"),
            ("<use href='#'/>", "nonempty"),
            ("<use href='#missing'/>", "missing ID"),
            ("<use href='other.svg#shape'/>", "external"),
            (
                "<use href='https://example.com/logo.svg#shape'/>",
                "external",
            ),
            (
                "<path id='a'/><rect id='a'/><use href='#a'/>",
                "duplicate ID",
            ),
            ("<defs id='a'/><use href='#a'/>", "supported vector element"),
        ] {
            assert!(check(body).unwrap_err().contains(expected), "{body}");
        }
    }

    #[test]
    fn self_ancestor_and_indirect_group_reference_cycles_are_rejected() {
        for body in [
            "<use id='self' href='#self'/>",
            "<g id='parent'><use href='#parent'/></g>",
            "<defs><g id='a'><g><use href='#b'/></g></g><g id='b'><use href='#a'/></g></defs><use href='#a'/>",
        ] {
            assert!(check(body).unwrap_err().contains("cyclic"), "{body}");
        }
    }

    #[test]
    fn exponential_clone_expansion_is_rejected_before_usvg_allocates_it() {
        let mut xml = "<defs><path id='n0' d='M0 0H1V1Z'/>".to_string();
        for n in 1..18 {
            xml.push_str(&format!(
                "<g id='n{n}'><use href='#n{}'/><use href='#n{}'/></g>",
                n - 1,
                n - 1
            ));
        }
        xml.push_str("</defs><use href='#n17'/>");
        assert!(check(&xml).unwrap_err().contains("100,000 nodes"));
    }

    #[test]
    fn source_and_nested_depth_limits_are_explicit() {
        let large = "<g/>".repeat(MAX_NODES);
        assert!(check(&large).unwrap_err().contains("XML nodes"));
        let deep = "<g>".repeat(MAX_DEPTH) + &"</g>".repeat(MAX_DEPTH);
        assert!(check(&deep).unwrap_err().contains("128 nested nodes"));
    }
}
