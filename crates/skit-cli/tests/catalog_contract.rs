use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use syn::{
    Attribute, Expr, ExprCall, ExprLit, Ident, ItemMod, Lit, Macro, Meta, Token,
    parse::{ParseStream, Parser as _},
    punctuated::Punctuated,
    visit::{self, Visit},
};

#[derive(Default)]
struct CatalogUses {
    message_templates: Vec<String>,
    human_templates: Vec<String>,
}

/// The arguments of a human output macro, past the style name it may carry.
///
/// A line may name the colour of its sense (`humanln!(Green: "...")`). The template is the
/// argument after that name, so the scan steps over it and reads the same literal either way.
fn macro_arguments(input: ParseStream<'_>) -> syn::Result<Punctuated<Expr, Token![,]>> {
    if input.peek(Ident) && input.peek2(Token![:]) {
        let _: Ident = input.parse()?;
        let _: Token![:] = input.parse()?;
    }
    Punctuated::parse_terminated(input)
}

impl<'ast> Visit<'ast> for CatalogUses {
    fn visit_item_mod(&mut self, node: &'ast ItemMod) {
        if has_test_configuration(&node.attrs) {
            return;
        }
        visit::visit_item_mod(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        if is_message_new(&node.func) {
            let template = node
                .args
                .first()
                .and_then(string_literal)
                .expect("Message::new needs a literal template");
            self.message_templates.push(template);
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_macro(&mut self, node: &'ast Macro) {
        let name = node
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string());
        if matches!(name.as_deref(), Some("humanln" | "humanerrln")) {
            let expressions = macro_arguments
                .parse2(node.tokens.clone())
                .expect("human output macro arguments must parse");
            let expression = expressions
                .first()
                .expect("a human output macro needs a template");
            self.human_templates.push(
                string_literal(expression).expect("a human output macro needs a literal template"),
            );
        }
        visit::visit_macro(self, node);
    }
}

fn has_test_configuration(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        matches!(
            &attribute.meta,
            Meta::List(list)
                if list.path.is_ident("cfg") && list.tokens.to_string().contains("test")
        )
    })
}

fn is_message_new(expression: &Expr) -> bool {
    let Expr::Path(path) = expression else {
        return false;
    };
    let mut segments = path.path.segments.iter().rev();
    segments
        .next()
        .is_some_and(|segment| segment.ident == "new")
        && segments
            .next()
            .is_some_and(|segment| segment.ident == "Message")
}

fn string_literal(expression: &Expr) -> Option<String> {
    let Expr::Lit(ExprLit {
        lit: Lit::Str(value),
        ..
    }) = expression
    else {
        return None;
    };
    Some(value.value())
}

fn rust_sources() -> Vec<PathBuf> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the CLI crate is inside the crates directory");
    let mut sources = Vec::new();
    let mut pending = fs::read_dir(crates)
        .expect("the crates directory is readable")
        .map(|item| item.expect("each crate is readable").path().join("src"))
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    while let Some(directory) = pending.pop() {
        for item in fs::read_dir(directory).expect("each source directory is readable") {
            let path = item.expect("each source item is readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|value| value == "rs") {
                sources.push(path);
            }
        }
    }
    sources
}

fn catalog_uses(source: &str) -> CatalogUses {
    let file = syn::parse_file(source).expect("each Rust source file must parse");
    let mut uses = CatalogUses::default();
    uses.visit_file(&file);
    uses
}

#[test]
fn every_shipped_message_template_has_a_complete_catalog_row() {
    let translated = skit_i18n::catalog()
        .iter()
        .map(|row| row.english)
        .collect::<BTreeSet<_>>();
    let mut templates = BTreeMap::<String, Vec<PathBuf>>::new();
    let sources = rust_sources();
    assert!(sources.len() > 20, "the source walk found too few files");
    for path in sources {
        let source = fs::read_to_string(&path).expect("each Rust source is UTF-8");
        for template in catalog_uses(&source).message_templates {
            templates.entry(template).or_default().push(path.clone());
        }
    }
    assert!(templates.len() > 100, "the scan found too few templates");
    let missing = templates
        .keys()
        .filter(|template| !translated.contains(template.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "add a complete catalog row for each template:\n{}",
        missing.join("\n")
    );
}

#[test]
fn every_cli_human_output_macro_has_a_complete_catalog_row() {
    let translated = skit_i18n::catalog()
        .iter()
        .map(|row| row.english)
        .collect::<BTreeSet<_>>();
    let source = include_str!("../src/cli.rs");
    let templates = catalog_uses(source).human_templates;
    assert!(
        templates.len() > 30,
        "the scan found too few human messages"
    );
    for template in templates {
        assert!(
            translated.contains(template.as_str()),
            "missing CLI translation template: {template}"
        );
    }
}

#[test]
fn syntax_parser_handles_raw_strings_and_braces_inside_test_modules() {
    let source = r####"
fn shipped() {
    let _ = Message::new(r#"raw \" text {}"#);
}

#[cfg(test)]
mod tests {
    const BRACE: &str = "}";
    fn ignored() { let _ = Message::new("test-only"); }
}
"####;
    let uses = catalog_uses(source);
    assert_eq!(uses.message_templates, ["raw \\\" text {}"]);
}
