//! The six CRUD operations an `on_*` attribute can register a handler for.

use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use syn::{Ident, Path};

/// The operation a hook attribute stands for.
///
/// Everything that differs between the six events lives here: the attribute
/// that names one, the registry method the expansion calls and the type a
/// handler may stop propagation with. Adding an event is a variant, the two
/// match arms the compiler then asks for, an entry in [`Operation::ALL`] and
/// the standalone attribute in the crate root; the tests below catch the two
/// the compiler cannot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Operation {
    Create,
    CreateMany,
    Update,
    UpdateMany,
    Delete,
    DeleteMany,
}

impl Operation {
    /// Every operation, so a name can be looked up against the one table that
    /// spells them and the tests can walk the whole set.
    pub(crate) const ALL: [Self; 6] = [
        Self::Create,
        Self::CreateMany,
        Self::Update,
        Self::UpdateMany,
        Self::Delete,
        Self::DeleteMany,
    ];

    /// The operation an attribute name registers, if it names one at all.
    pub(crate) fn from_attr_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.attr_name() == name)
    }

    /// The attribute that asks for this operation, as the user writes it.
    pub(crate) fn attr_name(self) -> &'static str {
        match self {
            Self::Create => "on_create",
            Self::CreateMany => "on_create_many",
            Self::Update => "on_update",
            Self::UpdateMany => "on_update_many",
            Self::Delete => "on_delete",
            Self::DeleteMany => "on_delete_many",
        }
    }

    /// The registry method the expansion calls, spanned at the attribute that
    /// asked for it so a client missing the method reports there.
    pub(crate) fn registry_method(self, span: Span) -> Ident {
        format_ident!("{}_with_priority", self.attr_name(), span = span)
    }

    /// What a handler for this operation can stop propagation with, which is
    /// what the delegate produces for it.
    pub(crate) fn result_type(self, model: &Path) -> TokenStream {
        match self {
            Self::Create => quote!(#model),
            Self::CreateMany | Self::Update | Self::DeleteMany => {
                quote!(::std::vec::Vec<#model>)
            }
            Self::Delete => quote!(::std::option::Option<#model>),
            Self::UpdateMany => quote!(::core::primitive::u64),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Operation;
    use proc_macro2::Span;

    /// The names have one owner: whatever `attr_name` writes is what the
    /// attribute is recognised by, so a new event cannot arrive half-named.
    #[test]
    fn every_operation_is_reachable_by_the_name_it_writes() {
        for operation in Operation::ALL {
            assert_eq!(
                Operation::from_attr_name(operation.attr_name()),
                Some(operation),
                "`{}` is not recognised as its own attribute",
                operation.attr_name()
            );
        }
    }

    #[test]
    fn operations_have_distinct_attributes_and_registry_methods() {
        let mut names: Vec<_> = Operation::ALL.iter().map(|op| op.attr_name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Operation::ALL.len());

        let mut methods: Vec<_> = Operation::ALL
            .iter()
            .map(|op| op.registry_method(Span::call_site()).to_string())
            .collect();
        methods.sort_unstable();
        methods.dedup();
        assert_eq!(methods.len(), Operation::ALL.len());
    }

    #[test]
    fn an_unknown_attribute_names_no_operation() {
        assert!(Operation::from_attr_name("on_upsert").is_none());
        assert!(Operation::from_attr_name("cfg").is_none());
    }
}
