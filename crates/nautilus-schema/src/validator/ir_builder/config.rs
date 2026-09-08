//! The `datasource` and `generator` blocks as the IR carries them.

use crate::validator::*;

impl SchemaValidator<'_> {
    pub(in crate::validator) fn build_datasource_ir(
        &self,
        datasource: &DatasourceDecl,
    ) -> Result<DatasourceIr> {
        let provider = Self::datasource_provider_value(datasource)?;
        let url = Self::datasource_url_value(datasource)?;
        let direct_url = Self::datasource_direct_url_value(datasource)?;
        let extensions = Self::datasource_extensions_value(datasource);
        let preserve_extensions = Self::datasource_preserve_extensions_value(datasource)?;
        let schemas = Self::datasource_schemas_value(datasource);

        Ok(DatasourceIr {
            name: datasource.name.value.clone(),
            provider,
            url,
            direct_url,
            extensions,
            schemas,
            preserve_extensions,
            span: datasource.span,
        })
    }

    /// Extracts, normalises and sorts the declared extensions.
    ///
    /// Assumes `validate_datasource_extensions` has already flagged structural
    /// problems: malformed entries are silently skipped here. For the
    /// structured `extension(name = ..., schema = ...)` form, the schema is
    /// preserved as `Some("…")`; the bare identifier and string-literal forms
    /// produce entries with `schema = None`.
    pub(in crate::validator) fn datasource_extensions_value(
        datasource: &DatasourceDecl,
    ) -> Vec<PostgresExtensionIr> {
        let Some(field) = datasource.find_field("extensions") else {
            return Vec::new();
        };
        let Expr::Array { elements, .. } = &field.value else {
            return Vec::new();
        };

        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut entries: Vec<PostgresExtensionIr> = elements
            .iter()
            .filter_map(|e| Self::parse_extension_entry(e).ok())
            .filter_map(|entry| {
                let name = entry.name.to_lowercase();
                if name.is_empty() || !seen.insert(name.clone()) {
                    return None;
                }
                Some(PostgresExtensionIr {
                    name,
                    schema: entry.schema,
                })
            })
            .collect();
        entries.sort();
        entries
    }

    pub(in crate::validator) fn build_generator_ir(
        &self,
        generator: &GeneratorDecl,
    ) -> Result<GeneratorIr> {
        let (provider, client_provider) = Self::generator_provider_info(generator)?;
        let output = Self::generator_output_value(generator)?;
        let interface = Self::generator_interface_kind(generator)?;
        let recursive_type_depth =
            Self::generator_recursive_type_depth(generator, client_provider, &provider)?;
        let java_package = Self::generator_java_package_value(generator, client_provider)?;
        let java_group_id = Self::generator_java_group_id_value(generator, client_provider)?;
        let java_artifact_id = Self::generator_java_artifact_id_value(generator, client_provider)?;
        let java_mode = Self::generator_java_mode_value(generator, client_provider)?;

        Ok(GeneratorIr {
            name: generator.name.value.clone(),
            provider,
            output,
            interface,
            recursive_type_depth,
            java_package,
            java_group_id,
            java_artifact_id,
            java_mode,
            span: generator.span,
        })
    }
}
