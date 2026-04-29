//! Shared helpers for extension-backed scalar wrapper generation.

use nautilus_schema::ir::{FieldIr, ResolvedFieldType, ScalarType, SchemaIr};
use std::collections::{BTreeSet, HashMap};
use tera::{Context, Tera};

type GeneratedFile = (String, String);
type GeneratedJsExtensionFiles = (Vec<GeneratedFile>, Vec<GeneratedFile>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionWireKind {
    String,
    Hstore,
    Vector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionRender {
    StringWrapper { value_variant: &'static str },
    Hstore,
    Vector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExtensionScalar {
    Citext,
    Hstore,
    Ltree,
    Geometry,
    Geography,
    Vector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionType {
    pub extension: &'static str,
    pub type_name: &'static str,
    pub scalar: ExtensionScalar,
    pub wire_kind: ExtensionWireKind,
    pub render: ExtensionRender,
}

#[derive(Debug, Clone, Default)]
pub struct ExtensionRegistry {
    declared: BTreeSet<String>,
}

impl ExtensionRegistry {
    pub fn from_schema(ir: &SchemaIr) -> Self {
        let declared = ir
            .datasource
            .as_ref()
            .map(|ds| {
                ds.extensions
                    .iter()
                    .map(|ext| ext.name.clone())
                    .collect::<BTreeSet<_>>()
            })
            .unwrap_or_default();
        Self { declared }
    }

    pub fn is_declared(&self, extension: &str) -> bool {
        self.declared.contains(extension)
    }

    pub fn type_for_scalar(&self, scalar: &ScalarType) -> Option<ExtensionType> {
        let ty = match scalar {
            ScalarType::Citext => CITEXT,
            ScalarType::Hstore => HSTORE,
            ScalarType::Ltree => LTREE,
            ScalarType::Geometry => GEOMETRY,
            ScalarType::Geography => GEOGRAPHY,
            ScalarType::Vector { .. } => VECTOR,
            _ => return None,
        };

        self.is_declared(ty.extension).then_some(ty)
    }

    pub fn type_for_field(&self, field: &FieldIr) -> Option<ExtensionType> {
        match &field.field_type {
            ResolvedFieldType::Scalar(scalar) => self.type_for_scalar(scalar),
            _ => None,
        }
    }

    pub fn active_extensions(&self) -> Vec<&'static str> {
        known_extensions()
            .into_iter()
            .filter(|ext| self.is_declared(ext))
            .collect()
    }

    /// All extension wrapper types that should be generated for the active
    /// extension set. PostGIS contributes both `Geometry` and `Geography`; all
    /// other extensions contribute a single wrapper type.
    pub fn active_types(&self) -> Vec<ExtensionType> {
        ALL_EXTENSION_TYPES
            .iter()
            .copied()
            .filter(|ty| self.is_declared(ty.extension))
            .collect()
    }

    pub fn has_active_types(&self) -> bool {
        !self.active_extensions().is_empty()
    }

    pub fn template_flags(&self) -> HashMap<String, bool> {
        known_extensions()
            .into_iter()
            .map(|extension| {
                (
                    format!("has_{extension}_extension"),
                    self.is_declared(extension),
                )
            })
            .collect()
    }
}

/// Single source of truth for the wrapper types codegen knows about. Adding a
/// new extension scalar means appending one entry here; `active_types` and
/// `active_extensions` derive from this list.
const ALL_EXTENSION_TYPES: &[ExtensionType] = &[CITEXT, HSTORE, LTREE, GEOMETRY, GEOGRAPHY, VECTOR];

const CITEXT: ExtensionType = ExtensionType {
    extension: "citext",
    type_name: "Citext",
    scalar: ExtensionScalar::Citext,
    wire_kind: ExtensionWireKind::String,
    render: ExtensionRender::StringWrapper {
        value_variant: "String",
    },
};
const HSTORE: ExtensionType = ExtensionType {
    extension: "hstore",
    type_name: "Hstore",
    scalar: ExtensionScalar::Hstore,
    wire_kind: ExtensionWireKind::Hstore,
    render: ExtensionRender::Hstore,
};
const LTREE: ExtensionType = ExtensionType {
    extension: "ltree",
    type_name: "Ltree",
    scalar: ExtensionScalar::Ltree,
    wire_kind: ExtensionWireKind::String,
    render: ExtensionRender::StringWrapper {
        value_variant: "String",
    },
};
const GEOMETRY: ExtensionType = ExtensionType {
    extension: "postgis",
    type_name: "Geometry",
    scalar: ExtensionScalar::Geometry,
    wire_kind: ExtensionWireKind::String,
    render: ExtensionRender::StringWrapper {
        value_variant: "Geometry",
    },
};
const GEOGRAPHY: ExtensionType = ExtensionType {
    extension: "postgis",
    type_name: "Geography",
    scalar: ExtensionScalar::Geography,
    wire_kind: ExtensionWireKind::String,
    render: ExtensionRender::StringWrapper {
        value_variant: "Geography",
    },
};
const VECTOR: ExtensionType = ExtensionType {
    extension: "vector",
    type_name: "Vector",
    scalar: ExtensionScalar::Vector,
    wire_kind: ExtensionWireKind::Vector,
    render: ExtensionRender::Vector,
};

impl ExtensionType {
    pub fn is_geometry(self) -> bool {
        matches!(self.scalar, ExtensionScalar::Geometry)
    }

    pub fn is_geography(self) -> bool {
        matches!(self.scalar, ExtensionScalar::Geography)
    }

    pub fn is_spatial(self) -> bool {
        self.is_geometry() || self.is_geography()
    }

    pub fn rust_type_path(self) -> String {
        format!(
            "crate::extensions::{}::types::{}",
            self.extension, self.type_name
        )
    }

    pub fn java_import(self, root_package: &str) -> String {
        format!(
            "{root_package}.extensions.{}.types.{}",
            self.extension, self.type_name
        )
    }

    pub fn input_alias(self) -> String {
        format!("{}Input", self.type_name)
    }

    pub fn python_raw_type(self) -> &'static str {
        match self.wire_kind {
            ExtensionWireKind::String => "str",
            ExtensionWireKind::Hstore => "HstoreValue",
            ExtensionWireKind::Vector => "List[float]",
        }
    }

    pub fn java_raw_type(self) -> &'static str {
        match self.wire_kind {
            ExtensionWireKind::String => "String",
            ExtensionWireKind::Hstore => "JsonSupport.Hstore",
            ExtensionWireKind::Vector => "List<Float>",
        }
    }

    pub fn python_filter_input(self) -> String {
        match self.wire_kind {
            ExtensionWireKind::String => format!("Union[{}, StringFilter]", self.input_alias()),
            ExtensionWireKind::Hstore => format!("Union[{}, HstoreFilter]", self.input_alias()),
            ExtensionWireKind::Vector => format!("Union[{}, VectorFilter]", self.input_alias()),
        }
    }

    pub fn ts_filter_input(self) -> String {
        match self.wire_kind {
            ExtensionWireKind::String => format!("{} | StringFilter", self.input_alias()),
            ExtensionWireKind::Hstore => format!("{} | HstoreFilter", self.input_alias()),
            ExtensionWireKind::Vector => format!("{} | VectorFilter", self.input_alias()),
        }
    }
}

pub(crate) fn python_input_type_for_extension(ty: ExtensionType) -> String {
    ty.input_alias()
}

pub(crate) fn ts_input_type_for_extension(ty: ExtensionType) -> String {
    ty.input_alias()
}

fn known_extensions() -> Vec<&'static str> {
    let mut seen = BTreeSet::new();
    ALL_EXTENSION_TYPES
        .iter()
        .map(|ty| ty.extension)
        .filter(|ext| seen.insert(*ext))
        .collect()
}

fn types_for_extension(extension: &str) -> impl Iterator<Item = ExtensionType> + '_ {
    ALL_EXTENSION_TYPES
        .iter()
        .copied()
        .filter(move |ty| ty.extension == extension)
}

static EXTENSION_TEMPLATES: std::sync::LazyLock<Tera> = std::sync::LazyLock::new(|| {
    let mut tera = Tera::default();
    tera.add_raw_templates(vec![
        (
            "rust/string_wrapper.tera",
            include_str!("../templates/rust/extensions/string_wrapper.tera"),
        ),
        (
            "rust/hstore_wrapper.tera",
            include_str!("../templates/rust/extensions/hstore_wrapper.tera"),
        ),
        (
            "rust/vector_wrapper.tera",
            include_str!("../templates/rust/extensions/vector_wrapper.tera"),
        ),
        (
            "python/string_wrapper_class.py.tera",
            include_str!("../templates/python/extensions/string_wrapper_class.py.tera"),
        ),
        (
            "python/hstore_wrapper.py.tera",
            include_str!("../templates/python/extensions/hstore_wrapper.py.tera"),
        ),
        (
            "python/vector_wrapper.py.tera",
            include_str!("../templates/python/extensions/vector_wrapper.py.tera"),
        ),
        (
            "js/string_wrapper.js.tera",
            include_str!("../templates/js/extensions/string_wrapper.js.tera"),
        ),
        (
            "js/string_wrapper.d.ts.tera",
            include_str!("../templates/js/extensions/string_wrapper.d.ts.tera"),
        ),
        (
            "js/hstore_wrapper.js.tera",
            include_str!("../templates/js/extensions/hstore_wrapper.js.tera"),
        ),
        (
            "js/hstore_wrapper.d.ts.tera",
            include_str!("../templates/js/extensions/hstore_wrapper.d.ts.tera"),
        ),
        (
            "js/vector_wrapper.js.tera",
            include_str!("../templates/js/extensions/vector_wrapper.js.tera"),
        ),
        (
            "js/vector_wrapper.d.ts.tera",
            include_str!("../templates/js/extensions/vector_wrapper.d.ts.tera"),
        ),
        (
            "java/string_wrapper.java.tera",
            include_str!("../templates/java/extensions/string_wrapper.java.tera"),
        ),
        (
            "java/hstore_wrapper.java.tera",
            include_str!("../templates/java/extensions/hstore_wrapper.java.tera"),
        ),
        (
            "java/vector_wrapper.java.tera",
            include_str!("../templates/java/extensions/vector_wrapper.java.tera"),
        ),
    ])
    .expect("embedded extension templates must parse");
    tera
});

fn render_ext(template: &str, ctx: &Context) -> String {
    crate::template::render(&EXTENSION_TEMPLATES, template, ctx)
}

pub fn generate_rust_extension_files(registry: &ExtensionRegistry) -> Vec<(String, String)> {
    let extensions = registry.active_extensions();
    if extensions.is_empty() {
        return Vec::new();
    }

    let mut files = Vec::new();
    let mut root_mod = String::from("//! Generated extension scalar types.\n\n");
    for ext in &extensions {
        root_mod.push_str(&format!("pub mod {ext};\n"));
        files.push((
            format!("extensions/{ext}/mod.rs"),
            "pub mod types;\n".to_string(),
        ));
        files.push((
            format!("extensions/{ext}/types.rs"),
            rust_types_for_extension(ext),
        ));
    }
    files.push(("extensions/mod.rs".to_string(), root_mod));
    files
}

fn rust_types_for_extension(extension: &str) -> String {
    let mut code = String::from("//! Generated extension scalar wrappers.\n\n");
    let sections = types_for_extension(extension)
        .map(|ty| match ty.render {
            ExtensionRender::StringWrapper { value_variant } => {
                render_rust_string_wrapper(ty, value_variant)
            }
            ExtensionRender::Hstore => render_ext("rust/hstore_wrapper.tera", &Context::new()),
            ExtensionRender::Vector => render_ext("rust/vector_wrapper.tera", &Context::new()),
        })
        .collect::<Vec<_>>();
    code.push_str(&sections.join("\n"));
    code
}

fn render_rust_string_wrapper(ty: ExtensionType, value_variant: &str) -> String {
    let match_pattern = if value_variant == "String" {
        "nautilus_core::Value::String(value)".to_string()
    } else {
        format!(
            "nautilus_core::Value::{value_variant}(value) | nautilus_core::Value::String(value)"
        )
    };
    let mut ctx = Context::new();
    ctx.insert("type_name", ty.type_name);
    ctx.insert("value_variant", value_variant);
    ctx.insert("match_pattern", &match_pattern);
    ctx.insert("error_name", ty.type_name);
    ctx.insert("is_geometry", &ty.is_geometry());
    ctx.insert("is_geography", &ty.is_geography());
    ctx.insert("is_spatial", &ty.is_spatial());
    render_ext("rust/string_wrapper.tera", &ctx)
}

pub fn generate_python_extension_files(registry: &ExtensionRegistry) -> Vec<(String, String)> {
    registry
        .active_extensions()
        .into_iter()
        .map(|extension| {
            (
                format!("{extension}/types.py"),
                python_types_for_extension(extension),
            )
        })
        .collect()
}

fn python_types_for_extension(extension: &str) -> String {
    let mut sections = Vec::new();
    let mut string_wrappers = Vec::new();

    let flush_string_wrappers =
        |sections: &mut Vec<String>, string_wrappers: &mut Vec<ExtensionType>| {
            if !string_wrappers.is_empty() {
                sections.push(render_python_string_wrappers(string_wrappers));
                string_wrappers.clear();
            }
        };

    for ty in types_for_extension(extension) {
        match ty.render {
            ExtensionRender::StringWrapper { .. } => string_wrappers.push(ty),
            ExtensionRender::Hstore => {
                flush_string_wrappers(&mut sections, &mut string_wrappers);
                sections.push(render_ext("python/hstore_wrapper.py.tera", &Context::new()));
            }
            ExtensionRender::Vector => {
                flush_string_wrappers(&mut sections, &mut string_wrappers);
                sections.push(render_ext("python/vector_wrapper.py.tera", &Context::new()));
            }
        }
    }

    flush_string_wrappers(&mut sections, &mut string_wrappers);
    sections.join("\n")
}

fn render_python_string_wrappers(types: &[ExtensionType]) -> String {
    let mut code =
        "from __future__ import annotations\n\nfrom dataclasses import dataclass\nfrom typing import Any, NotRequired, TypedDict, Union\n\n"
            .to_string();
    for ty in types {
        let mut ctx = Context::new();
        ctx.insert("type_name", ty.type_name);
        ctx.insert("is_geometry", &ty.is_geometry());
        ctx.insert("is_geography", &ty.is_geography());
        ctx.insert("is_spatial", &ty.is_spatial());
        code.push_str(&render_ext("python/string_wrapper_class.py.tera", &ctx));
        code.push('\n');
    }
    let all_names = types
        .iter()
        .flat_map(|ty| {
            [
                format!("{:?}", ty.type_name),
                format!("{:?}", ty.input_alias()),
            ]
        })
        .collect::<Vec<_>>()
        .join(", ");
    code.push_str(&format!("\n__all__ = [{all_names}]\n"));
    code
}

pub fn generate_js_extension_files(registry: &ExtensionRegistry) -> GeneratedJsExtensionFiles {
    let mut js_files = Vec::new();
    let mut dts_files = Vec::new();
    for extension in registry.active_extensions() {
        js_files.push((
            format!("extensions/{extension}/types.js"),
            js_types_for_extension(extension),
        ));
        dts_files.push((
            format!("extensions/{extension}/types.d.ts"),
            ts_types_for_extension(extension),
        ));
    }
    (js_files, dts_files)
}

fn js_types_for_extension(extension: &str) -> String {
    types_for_extension(extension)
        .map(|ty| match ty.render {
            ExtensionRender::StringWrapper { .. } => render_js_string_wrapper(ty),
            ExtensionRender::Hstore => render_ext("js/hstore_wrapper.js.tera", &Context::new()),
            ExtensionRender::Vector => render_ext("js/vector_wrapper.js.tera", &Context::new()),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn ts_types_for_extension(extension: &str) -> String {
    types_for_extension(extension)
        .map(|ty| match ty.render {
            ExtensionRender::StringWrapper { .. } => render_ts_string_wrapper(ty),
            ExtensionRender::Hstore => render_ext("js/hstore_wrapper.d.ts.tera", &Context::new()),
            ExtensionRender::Vector => render_ext("js/vector_wrapper.d.ts.tera", &Context::new()),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_js_string_wrapper(ty: ExtensionType) -> String {
    let mut ctx = Context::new();
    ctx.insert("type_name", ty.type_name);
    ctx.insert("is_geometry", &ty.is_geometry());
    ctx.insert("is_geography", &ty.is_geography());
    ctx.insert("is_spatial", &ty.is_spatial());
    render_ext("js/string_wrapper.js.tera", &ctx)
}

fn render_ts_string_wrapper(ty: ExtensionType) -> String {
    let mut ctx = Context::new();
    ctx.insert("type_name", ty.type_name);
    ctx.insert("is_geometry", &ty.is_geometry());
    ctx.insert("is_geography", &ty.is_geography());
    ctx.insert("is_spatial", &ty.is_spatial());
    render_ext("js/string_wrapper.d.ts.tera", &ctx)
}

pub fn generate_java_extension_files(
    registry: &ExtensionRegistry,
    root_package: &str,
) -> Vec<(String, String)> {
    let mut files = Vec::new();
    for extension in registry.active_extensions() {
        for ty in types_for_extension(extension) {
            let code = match ty.render {
                ExtensionRender::StringWrapper { .. } => {
                    render_java_string_wrapper(root_package, extension, ty)
                }
                ExtensionRender::Hstore => render_java_hstore_wrapper(root_package, extension),
                ExtensionRender::Vector => render_java_vector_wrapper(root_package, extension),
            };
            files.push(java_extension_file(
                root_package,
                extension,
                ty.type_name,
                code,
            ));
        }
    }
    files
}

fn java_extension_file(
    root_package: &str,
    extension: &str,
    type_name: &str,
    code: String,
) -> (String, String) {
    let package_path = root_package.replace('.', "/");
    (
        format!("src/main/java/{package_path}/extensions/{extension}/types/{type_name}.java"),
        code,
    )
}

fn render_java_string_wrapper(root_package: &str, extension: &str, ty: ExtensionType) -> String {
    let mut ctx = Context::new();
    ctx.insert("root_package", root_package);
    ctx.insert("extension", extension);
    ctx.insert("type_name", ty.type_name);
    ctx.insert("is_geometry", &ty.is_geometry());
    ctx.insert("is_geography", &ty.is_geography());
    ctx.insert("is_spatial", &ty.is_spatial());
    render_ext("java/string_wrapper.java.tera", &ctx)
}

fn render_java_hstore_wrapper(root_package: &str, extension: &str) -> String {
    let mut ctx = Context::new();
    ctx.insert("root_package", root_package);
    ctx.insert("extension", extension);
    render_ext("java/hstore_wrapper.java.tera", &ctx)
}

fn render_java_vector_wrapper(root_package: &str, extension: &str) -> String {
    let mut ctx = Context::new();
    ctx.insert("root_package", root_package);
    ctx.insert("extension", extension);
    render_ext("java/vector_wrapper.java.tera", &ctx)
}
