//! Model files share the facade's Rust module through `include!`, preserving
//! private access and public type paths while separating source responsibilities.

use anyhow::Result;
use heck::ToSnakeCase;
use nautilus_schema::ir::SchemaIr;
use std::collections::HashMap;

use super::{model_context, templates::render};
use crate::{extension_types::ExtensionRegistry, GeneratedFile};

pub(crate) struct ModelFiles {
    pub(crate) facades: HashMap<String, String>,
    pub(crate) parts: Vec<GeneratedFile>,
}

pub(crate) fn generate_model_files(
    ir: &SchemaIr,
    is_async: bool,
    extensions: &ExtensionRegistry,
) -> Result<ModelFiles> {
    let mut generated = ModelFiles {
        facades: HashMap::new(),
        parts: Vec::new(),
    };

    for (name, model) in &ir.models {
        let mut context = model_context(model, ir, is_async, extensions);
        let directory = name.to_snake_case();
        let mut files = Vec::new();

        for (file, templates) in [
            (
                "model",
                &[
                    "model_struct.tera",
                    "column_impl.tera",
                    "columns_struct.tera",
                ][..],
            ),
            ("decode", &["model/decode.tera", "from_row_impl.tera"]),
            ("nested_input", &["delegate/nested_input.tera"]),
            ("input", &["delegate/write_input.tera"]),
            ("aggregate_input", &["delegate/aggregate_input.tera"]),
            (
                "aggregate_output",
                &[
                    "delegate/aggregate_output.tera",
                    "delegate/aggregate_decode.tera",
                ],
            ),
            ("write_filter", &["delegate/write_filter.tera"]),
            ("ordering", &["read/ordering.tera"]),
            ("read_builder", &["read/builder.tera"]),
            ("read_execute", &["read/execute.tera"]),
        ] {
            let mut code = String::new();
            for template in templates {
                code.push_str(&render(template, &context)?);
                code.push('\n');
            }
            add_part(&mut generated.parts, &mut files, &directory, file, code);
        }

        let state = render("delegate/state.tera", &context)? + "}\n";
        add_part(
            &mut generated.parts,
            &mut files,
            &directory,
            "delegate",
            state,
        );

        for (file, operations, builder) in [
            ("read", &["read"][..], None),
            ("projection", &["projection"], None),
            ("stream", &["stream"], None),
            ("create", &["create"], Some("create.tera")),
            ("create_many", &["create_many"], Some("create_many.tera")),
            ("update", &["update", "update_many"], Some("update.tera")),
            (
                "delete",
                &["delete", "delete_many", "delete_many_count"],
                Some("delete.tera"),
            ),
            ("aggregate", &["count", "aggregate"], None),
            ("upsert", &["upsert"], None),
        ] {
            let mut body = String::new();
            for operation in operations {
                body.push_str(&render(&format!("delegate/{operation}.tera"), &context)?);
            }
            if body.trim().is_empty() {
                continue;
            }
            context.insert("body", &body);
            let mut code = render("delegate/operation.tera", &context)?;
            if let Some(builder) = builder {
                code.push_str(&render(builder, &context)?);
            }
            add_part(&mut generated.parts, &mut files, &directory, file, code);
        }

        context.insert("files", &files);
        generated
            .facades
            .insert(name.clone(), render("model/facade.tera", &context)?);
    }

    generated.parts.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(generated)
}

fn add_part(
    parts: &mut Vec<GeneratedFile>,
    files: &mut Vec<String>,
    directory: &str,
    name: &str,
    code: String,
) {
    if !code.trim().is_empty() {
        let path = format!("{directory}/{name}.rs");
        files.push(path.clone());
        parts.push((path, code));
    }
}

#[cfg(test)]
mod tests {
    use super::generate_model_files;
    use crate::extension_types::ExtensionRegistry;
    use nautilus_schema::validate_schema_source;

    #[test]
    fn rust_model_file_layout() {
        let ir = validate_schema_source(include_str!(
            "../../tests/fixtures/schemas/user_post.nautilus"
        ))
        .unwrap()
        .ir;
        let files = generate_model_files(&ir, true, &ExtensionRegistry::from_schema(&ir)).unwrap();
        let mut facades: Vec<_> = files.facades.into_iter().collect();
        facades.sort_by(|left, right| left.0.cmp(&right.0));
        let mut layout = String::new();
        for (model, code) in facades {
            layout.push_str(&format!("{model}\n{code}\n"));
        }
        for (path, _) in files.parts {
            layout.push_str(&format!("{path}\n"));
        }
        insta::with_settings!({snapshot_path => "../../tests/snapshots"}, {
            insta::assert_snapshot!(layout);
        });
    }
}
