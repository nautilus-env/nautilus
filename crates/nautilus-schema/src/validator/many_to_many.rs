//! Implicit many-to-many relations and the join tables that carry them.
//!
//! A relation declared as an array on both sides — `Post.tags Tag[]` against
//! `Tag.posts Post[]` — has nowhere to put a foreign key. Rather than make the
//! user write the link table by hand, Nautilus synthesises one and records it
//! on both relation fields, so migrations create it and the engine reads and
//! writes the relation through it.

use super::*;

/// Column of the join table belonging to the side that sorts first.
const JOIN_COLUMN_A: &str = "A";
/// Column of the join table belonging to the side that sorts second.
const JOIN_COLUMN_B: &str = "B";
/// Relation field carrying the foreign key of [`JOIN_COLUMN_A`].
const JOIN_LINK_A: &str = "A_ref";
/// Relation field carrying the foreign key of [`JOIN_COLUMN_B`].
const JOIN_LINK_B: &str = "B_ref";

/// One side of an implicit many-to-many: the model that declares the array
/// field, and the field itself.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Endpoint {
    model: String,
    field: String,
}

/// The two ends of one implicit many-to-many relation, ordered so that `first`
/// is always the `A` side.
struct Pairing {
    first: Endpoint,
    second: Endpoint,
    relation_name: Option<String>,
    span: Span,
}

/// Link every implicit many-to-many relation in `ir` to a synthesised join
/// table, adding those tables to the schema.
///
/// Runs after every model is built, because deciding that a relation is an
/// implicit many-to-many needs both of its ends resolved.
pub(super) fn link_implicit_many_to_many(ir: &mut SchemaIr) -> Result<()> {
    for pairing in collect_pairings(ir)? {
        let table = join_table_name(&pairing);
        if ir.models.contains_key(&table) {
            return Err(SchemaError::Validation(
                format!(
                    "The many-to-many relation between '{}' and '{}' needs a join table called '{}', but a model of that name already exists. Rename it, or name the relation with @relation(name: ...) on both sides.",
                    pairing.first.model, pairing.second.model, table
                ),
                pairing.span,
            ));
        }

        let first_reference = single_primary_key(ir, &pairing.first.model, &pairing)?;
        let second_reference = single_primary_key(ir, &pairing.second.model, &pairing)?;
        let join_model =
            build_join_model(ir, &table, &pairing, &first_reference, &second_reference)?;

        set_join(
            ir,
            &pairing.first,
            ManyToManyJoinIr {
                table: table.clone(),
                self_column: JOIN_COLUMN_A.to_string(),
                target_column: JOIN_COLUMN_B.to_string(),
                self_reference: first_reference.clone(),
                target_reference: second_reference.clone(),
            },
        );
        set_join(
            ir,
            &pairing.second,
            ManyToManyJoinIr {
                table: table.clone(),
                self_column: JOIN_COLUMN_B.to_string(),
                target_column: JOIN_COLUMN_A.to_string(),
                self_reference: second_reference,
                target_reference: first_reference,
            },
        );

        ir.models.insert(table, join_model);
    }

    Ok(())
}

/// Find every pair of array relation fields that point at each other while
/// neither declares a foreign key.
///
/// The result is sorted by the `A` side, so a schema always produces the same
/// join tables in the same order however its models happen to be hashed.
fn collect_pairings(ir: &SchemaIr) -> Result<Vec<Pairing>> {
    let mut pairings: Vec<Pairing> = Vec::new();
    let mut paired: HashSet<Endpoint> = HashSet::new();

    let mut models: Vec<&ModelIr> = ir.models.values().collect();
    models.sort_by(|a, b| a.logical_name.cmp(&b.logical_name));

    for model in models {
        for field in &model.fields {
            let ResolvedFieldType::Relation(relation) = &field.field_type else {
                continue;
            };
            if !is_dangling_array(field, relation) {
                continue;
            }

            let this = Endpoint {
                model: model.logical_name.clone(),
                field: field.logical_name.clone(),
            };
            if paired.contains(&this) {
                continue;
            }

            let target = ir.models.get(&relation.target_model).ok_or_else(|| {
                SchemaError::Validation(
                    format!(
                        "Relation field '{}.{}' targets unknown model '{}'",
                        model.logical_name, field.logical_name, relation.target_model
                    ),
                    field.span,
                )
            })?;

            let Some(opposite) = opposite_array_field(target, &this, relation)? else {
                continue;
            };

            let other = Endpoint {
                model: target.logical_name.clone(),
                field: opposite.logical_name.clone(),
            };
            paired.insert(this.clone());
            paired.insert(other.clone());

            let (first, second) = if this <= other {
                (this, other)
            } else {
                (other, this)
            };
            pairings.push(Pairing {
                first,
                second,
                relation_name: relation.name.clone(),
                span: field.span,
            });
        }
    }

    pairings.sort_by(|a, b| a.first.cmp(&b.first));
    Ok(pairings)
}

/// Whether a field is an array relation that names no foreign key of its own.
fn is_dangling_array(field: &FieldIr, relation: &RelationIr) -> bool {
    field.is_array && relation.fields.is_empty() && relation.references.is_empty()
}

/// [`is_dangling_array`] for a field whose relation has not been matched yet.
fn dangling(field: &FieldIr) -> bool {
    match &field.field_type {
        ResolvedFieldType::Relation(relation) => is_dangling_array(field, relation),
        _ => false,
    }
}

/// Find the field on `target` that closes the relation `origin` opens.
///
/// Answers `None` whenever the pairing is not an implicit many-to-many — most
/// often because the opposite side holds a foreign key, which makes it an
/// ordinary one-to-many that needs nothing synthesised.
fn opposite_array_field<'a>(
    target: &'a ModelIr,
    origin: &Endpoint,
    relation: &RelationIr,
) -> Result<Option<&'a FieldIr>> {
    let candidates: Vec<&FieldIr> = target
        .fields
        .iter()
        .filter(|candidate| {
            let ResolvedFieldType::Relation(other) = &candidate.field_type else {
                return false;
            };
            if other.target_model != origin.model {
                return false;
            }
            if target.logical_name == origin.model && candidate.logical_name == origin.field {
                return false;
            }
            match relation.name.as_deref() {
                Some(name) => other.name.as_deref() == Some(name),
                None => other.name.is_none(),
            }
        })
        .collect();

    if candidates.len() > 1 && candidates.iter().all(|candidate| dangling(candidate)) {
        return Err(SchemaError::Validation(
            format!(
                "Relation field '{}.{}' could close a many-to-many with more than one field on model '{}': {}. Name each relation with @relation(name: ...) on both of its ends.",
                origin.model,
                origin.field,
                target.logical_name,
                candidates
                    .iter()
                    .map(|candidate| format!("'{}'", candidate.logical_name))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            candidates[0].span,
        ));
    }

    let [opposite] = candidates.as_slice() else {
        return Ok(None);
    };
    if !dangling(opposite) {
        return Ok(None);
    }

    if target.is_view {
        return Err(SchemaError::Validation(
            format!(
                "Relation field '{}.{}' is a many-to-many with view '{}'. A view is read-only, so Nautilus cannot create the join table the relation needs.",
                origin.model, origin.field, target.logical_name
            ),
            opposite.span,
        ));
    }

    Ok(Some(opposite))
}

/// The join-table name: `_<relation name>` when the relation is named,
/// `_<A model>To<B model>` otherwise.
fn join_table_name(pairing: &Pairing) -> String {
    match &pairing.relation_name {
        Some(name) => format!("_{}", name),
        None => format!("_{}To{}", pairing.first.model, pairing.second.model),
    }
}

/// The logical name of the model's single primary-key field.
///
/// An implicit many-to-many stores one key per side, so a composite primary key
/// has no column to land in; that schema has to declare the join table itself.
fn single_primary_key(ir: &SchemaIr, model_name: &str, pairing: &Pairing) -> Result<String> {
    let model = expect_model(ir, model_name);

    match model.primary_key.fields().as_slice() {
        [name] => Ok((*name).to_string()),
        _ => Err(SchemaError::Validation(
            format!(
                "The many-to-many relation between '{}' and '{}' needs a single-field primary key on '{}'. Declare the join table as a model of its own instead.",
                pairing.first.model, pairing.second.model, model_name
            ),
            model.span,
        )),
    }
}

/// Build the join table: one key column per side, a composite primary key over
/// the two, a foreign key each, and an index on `B` so the relation reads as
/// cheaply from either end.
fn build_join_model(
    ir: &SchemaIr,
    table: &str,
    pairing: &Pairing,
    first_reference: &str,
    second_reference: &str,
) -> Result<ModelIr> {
    let fields = vec![
        key_column(ir, &pairing.first.model, first_reference, JOIN_COLUMN_A)?,
        key_column(ir, &pairing.second.model, second_reference, JOIN_COLUMN_B)?,
        link_field(
            &pairing.first.model,
            JOIN_COLUMN_A,
            first_reference,
            JOIN_LINK_A,
        ),
        link_field(
            &pairing.second.model,
            JOIN_COLUMN_B,
            second_reference,
            JOIN_LINK_B,
        ),
    ];

    Ok(ModelIr {
        logical_name: table.to_string(),
        db_name: table.to_string(),
        schema: expect_model(ir, &pairing.first.model).schema.clone(),
        fields,
        primary_key: PrimaryKeyIr::Composite(vec![
            JOIN_COLUMN_A.to_string(),
            JOIN_COLUMN_B.to_string(),
        ]),
        unique_constraints: Vec::new(),
        indexes: vec![IndexIr {
            fields: vec![JOIN_COLUMN_B.to_string()],
            kind: IndexKind::Default,
            name: None,
            map: Some(format!("{}_{}_index", table, JOIN_COLUMN_B)),
            predicate: None,
        }],
        check_constraints: Vec::new(),
        is_ignored: false,
        is_view: false,
        is_join_table: true,
        span: pairing.span,
    })
}

/// A join-table key column, typed after the primary key it points at.
fn key_column(ir: &SchemaIr, model_name: &str, reference: &str, column: &str) -> Result<FieldIr> {
    let model = expect_model(ir, model_name);
    let referenced = model.find_field(reference).ok_or_else(|| {
        SchemaError::Validation(
            format!(
                "Model '{}' declares primary key '{}', but has no such field",
                model_name, reference
            ),
            model.span,
        )
    })?;

    Ok(FieldIr {
        logical_name: column.to_string(),
        db_name: column.to_string(),
        field_type: referenced.field_type.clone(),
        is_required: true,
        is_array: false,
        storage_strategy: None,
        default_value: None,
        is_unique: false,
        is_updated_at: false,
        computed: None,
        check: None,
        is_ignored: false,
        span: model.span,
    })
}

/// The relation field that turns one join-table column into a foreign key.
///
/// Deleting either end takes its links with it: a link to a row that is gone is
/// not a relation anyone can observe, only a constraint violation waiting for
/// the next write.
fn link_field(target_model: &str, column: &str, reference: &str, name: &str) -> FieldIr {
    FieldIr {
        logical_name: name.to_string(),
        db_name: name.to_string(),
        field_type: ResolvedFieldType::Relation(RelationIr {
            name: None,
            target_model: target_model.to_string(),
            fields: vec![column.to_string()],
            references: vec![reference.to_string()],
            on_delete: Some(ReferentialAction::Cascade),
            on_update: Some(ReferentialAction::Cascade),
            join: None,
        }),
        is_required: true,
        is_array: false,
        storage_strategy: None,
        default_value: None,
        is_unique: false,
        is_updated_at: false,
        computed: None,
        check: None,
        is_ignored: false,
        span: Span::new(0, 0),
    }
}

fn expect_model<'a>(ir: &'a SchemaIr, name: &str) -> &'a ModelIr {
    ir.models
        .get(name)
        .expect("pairing endpoints name models taken from this schema")
}

fn set_join(ir: &mut SchemaIr, endpoint: &Endpoint, join: ManyToManyJoinIr) {
    let Some(model) = ir.models.get_mut(&endpoint.model) else {
        return;
    };
    let Some(field) = model
        .fields
        .iter_mut()
        .find(|field| field.logical_name == endpoint.field)
    else {
        return;
    };
    if let ResolvedFieldType::Relation(relation) = &mut field.field_type {
        relation.join = Some(join);
    }
}
