//! Embedded Java templates and rendering.

use anyhow::Result;
use tera::{Context, Tera};

static JAVA_TEMPLATES: std::sync::LazyLock<Tera> = std::sync::LazyLock::new(|| {
    let mut tera = Tera::default();
    tera.add_raw_templates(vec![
        (
            "java_pom.tera",
            include_str!("../../../templates/java/pom.xml.tera"),
        ),
        (
            "java_enum.tera",
            include_str!("../../../templates/java/enum.java.tera"),
        ),
        (
            "java_composite.tera",
            include_str!("../../../templates/java/composite.java.tera"),
        ),
        (
            "java_model.tera",
            include_str!("../../../templates/java/model.java.tera"),
        ),
        (
            "java_projection.tera",
            include_str!("../../../templates/java/projection.java.tera"),
        ),
        (
            "java_delegate.tera",
            include_str!("../../../templates/java/delegate.java.tera"),
        ),
        (
            "_dsl_macros.tera",
            include_str!("../../../templates/java/_dsl_macros.tera"),
        ),
        (
            "java_dsl.tera",
            include_str!("../../../templates/java/dsl.java.tera"),
        ),
        (
            "java_transaction_client.tera",
            include_str!("../../../templates/java/transaction_client.java.tera"),
        ),
        (
            "java_nautilus.tera",
            include_str!("../../../templates/java/nautilus.java.tera"),
        ),
        (
            "java_nautilus_model.tera",
            include_str!("../../../templates/java/NautilusModel.java.tera"),
        ),
        (
            "java_sort_order.tera",
            include_str!("../../../templates/java/SortOrder.java.tera"),
        ),
        (
            "java_filters.tera",
            include_str!("../../../templates/java/Filters.java.tera"),
        ),
        (
            "java_nautilus_options.tera",
            include_str!("../../../templates/java/NautilusOptions.java.tera"),
        ),
        (
            "java_isolation_level.tera",
            include_str!("../../../templates/java/IsolationLevel.java.tera"),
        ),
        (
            "java_transaction_options.tera",
            include_str!("../../../templates/java/TransactionOptions.java.tera"),
        ),
        (
            "java_transaction_batch_op.tera",
            include_str!("../../../templates/java/TransactionBatchOperation.java.tera"),
        ),
        (
            "java_event_phase.tera",
            include_str!("../../../templates/java/events/EventPhase.java.tera"),
        ),
        (
            "java_event_context.tera",
            include_str!("../../../templates/java/events/CrudEventContext.java.tera"),
        ),
        (
            "java_stop_propagation.tera",
            include_str!("../../../templates/java/events/StopPropagation.java.tera"),
        ),
        (
            "java_on_create.tera",
            include_str!("../../../templates/java/events/OnCreate.java.tera"),
        ),
        (
            "java_on_create_many.tera",
            include_str!("../../../templates/java/events/OnCreateMany.java.tera"),
        ),
        (
            "java_on_update.tera",
            include_str!("../../../templates/java/events/OnUpdate.java.tera"),
        ),
        (
            "java_on_update_many.tera",
            include_str!("../../../templates/java/events/OnUpdateMany.java.tera"),
        ),
        (
            "java_on_delete.tera",
            include_str!("../../../templates/java/events/OnDelete.java.tera"),
        ),
        (
            "java_on_delete_many.tera",
            include_str!("../../../templates/java/events/OnDeleteMany.java.tera"),
        ),
        (
            "java_rt_wire_serializable.tera",
            include_str!("../../../templates/java/runtime/WireSerializable.java.tera"),
        ),
        (
            "java_rt_rpc_caller.tera",
            include_str!("../../../templates/java/runtime/RpcCaller.java.tera"),
        ),
        (
            "java_rt_global_registry.tera",
            include_str!("../../../templates/java/runtime/GlobalNautilusRegistry.java.tera"),
        ),
        (
            "java_rt_nautilus_exception.tera",
            include_str!("../../../templates/java/runtime/NautilusException.java.tera"),
        ),
        (
            "java_rt_protocol_exception.tera",
            include_str!("../../../templates/java/runtime/ProtocolException.java.tera"),
        ),
        (
            "java_rt_handshake_exception.tera",
            include_str!("../../../templates/java/runtime/HandshakeException.java.tera"),
        ),
        (
            "java_rt_transaction_exception.tera",
            include_str!("../../../templates/java/runtime/TransactionException.java.tera"),
        ),
        (
            "java_rt_not_found_exception.tera",
            include_str!("../../../templates/java/runtime/NotFoundException.java.tera"),
        ),
        (
            "java_rt_json_support.tera",
            include_str!("../../../templates/java/runtime/JsonSupport.java.tera"),
        ),
        (
            "java_rt_engine_process.tera",
            include_str!("../../../templates/java/runtime/EngineProcess.java.tera"),
        ),
        (
            "java_rt_base_client.tera",
            include_str!("../../../templates/java/runtime/BaseNautilusClient.java.tera"),
        ),
        (
            "java_rt_base_tx_client.tera",
            include_str!("../../../templates/java/runtime/BaseTransactionClient.java.tera"),
        ),
        (
            "java_rt_abstract_delegate.tera",
            include_str!("../../../templates/java/runtime/AbstractDelegate.java.tera"),
        ),
        (
            "java_rt_event_registry.tera",
            include_str!("../../../templates/java/runtime/EventRegistry.java.tera"),
        ),
    ])
    .expect("embedded Java templates must parse");
    tera
});

pub(super) fn render(template: &str, context: &Context) -> Result<String> {
    crate::template::render(&JAVA_TEMPLATES, template, context)
}

/// Render a template that only requires `package_name`.
pub(super) fn render_pkg(template: &str, package_name: &str) -> Result<String> {
    let mut ctx = Context::new();
    crate::template::insert_protocol_version(&mut ctx);
    ctx.insert("package_name", package_name);
    render(template, &ctx)
}
