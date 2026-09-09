use super::support::{generated_java_file, generated_named_file, generated_python_file, validate};
use nautilus_codegen::java::generate_java_client;
use nautilus_codegen::js::{generate_all_js_models, generate_js_client, js_runtime_files};
use nautilus_codegen::python::{generate_all_python_models, python_runtime_files};

const JAVA_CLIENT_SCHEMA: &str = include_str!("../fixtures/schemas/java_client.nautilus");
const USER_SCHEMA: &str = include_str!("../fixtures/schemas/user.nautilus");

#[test]
fn test_generated_python_and_js_cud_event_api() {
    let ir = validate(USER_SCHEMA);

    let py_models = generate_all_python_models(&ir, true, 0)
        .expect("generate_all_python_models should succeed");
    let py_user = generated_python_file(&py_models, "user.py");
    let py_runtime = python_runtime_files();
    let py_events_runtime = generated_named_file(&py_runtime, "_events.py");

    assert!(
        py_user.contains("__nautilus_model_name__")
            && py_user.contains("def onDelete")
            && py_user.contains("UserCreateEventContext = CrudEventContext")
            && py_user.contains("Callable[[\"UserCreateEventContext\"], Any]")
            && py_user.contains("model_event_decorator(cls, \"delete\"")
            && py_user.contains("priority: int = 0")
            && py_user.contains("run_crud_event(_before_ctx)")
            && py_user.contains(
                "resolve_stop_result(_stop, default_cud_result(\"deleteMany\", return_data))"
            ),
        "expected generated Python model to expose and run CUD events:\n{py_user}"
    );
    assert!(
        py_events_runtime.contains("class StopPropagation")
            && py_events_runtime.contains("class EventPhase")
            && py_events_runtime.contains("class CrudEventContext(")
            && py_events_runtime.contains("Generic[ModelT, OperationT")
            && py_events_runtime.contains("CrudEventHandler")
            && py_events_runtime.contains("handle_stop_propagation: bool = True")
            && py_events_runtime.contains("normalize_event_priority")
            && py_events_runtime
                .contains("handlers.sort(key=lambda registered: registered.priority, reverse=True)"),
        "expected Python event runtime to expose phases, context, and StopPropagation:\n{py_events_runtime}"
    );

    let (js_models, js_dts_models) =
        generate_all_js_models(&ir).expect("generate_all_js_models should succeed");
    let js_user = generated_named_file(&js_models, "user.js");
    let js_user_dts = generated_named_file(&js_dts_models, "user.d.ts");
    let (js_client, js_client_dts) = generate_js_client(&ir.models, "schema.nautilus")
        .expect("generate_js_client should succeed");
    let js_runtime = js_runtime_files();
    let js_events_runtime = generated_named_file(&js_runtime, "_events.js");
    let js_events_dts = generated_named_file(&js_runtime, "_events.d.ts");

    assert!(
        js_user.contains("export const User = createModelEvents('User')")
            && js_user.contains("runCrudEvent(_UserEventContext")
            && js_user
                .contains("resolveStopResult(stop, defaultCrudResult('deleteMany', returnData))"),
        "expected generated JS model to expose and run CUD events:\n{js_user}"
    );
    assert!(
        js_user_dts.contains("export declare const User: ModelEventToken")
            && js_user_dts.contains("UserCreateEventContext = CrudEventContext")
            && js_user_dts.contains("UserEventContexts extends ModelEventContexts")
            && js_user_dts.contains("data: UserCreateInput"),
        "expected generated JS declarations to type the model event token:\n{js_user_dts}"
    );
    assert!(
        js_client.contains("export { EventPhase, StopPropagation }")
            && js_client.contains("export * from './models/index.js'")
            && js_client_dts.contains("CrudEventContext")
            && js_client_dts.contains("EventPhaseValue")
            && js_client_dts.contains("ModelEventToken"),
        "expected JS root client to re-export events and model tokens:\n{js_client}\n\n{js_client_dts}"
    );
    assert!(
        js_events_runtime.contains("class StopPropagation")
            && js_events_runtime.contains("createModelEvents")
            && js_events_runtime.contains("runCrudEvent")
            && js_events_runtime.contains("model_name")
            && js_events_runtime.contains("normalizeEventPriority")
            && js_events_runtime
                .contains("handlers.sort((left, right) => right.priority - left.priority)")
            && js_events_dts.contains("result?: TResult")
            && js_events_dts.contains("interface EventPriorityOptions")
            && js_events_dts.contains("interface ModelEventToken"),
        "expected JS event runtime to expose phases, context, and StopPropagation:\n{js_events_runtime}\n\n{js_events_dts}"
    );
}

#[test]
fn test_generated_java_cud_event_api() {
    let ir = validate(JAVA_CLIENT_SCHEMA);

    let java_files =
        generate_java_client(&ir, "schema.nautilus", false).expect("generate_java_client failed");
    let on_create = generated_java_file(&java_files, "events/OnCreate.java");
    let context = generated_java_file(&java_files, "events/CrudEventContext.java");
    let stop = generated_java_file(&java_files, "events/StopPropagation.java");
    let options = generated_java_file(&java_files, "client/NautilusOptions.java");
    let nautilus = generated_java_file(&java_files, "client/Nautilus.java");
    let delegate = generated_java_file(&java_files, "client/UserDelegate.java");
    let base_delegate = generated_java_file(&java_files, "internal/AbstractDelegate.java");
    let registry = generated_java_file(&java_files, "internal/EventRegistry.java");

    assert!(
        on_create.contains("@Retention(RetentionPolicy.RUNTIME)")
            && on_create.contains("public @interface OnCreate")
            && on_create.contains("Class<?> value();")
            && on_create.contains("EventPhase phase() default EventPhase.BEFORE;")
            && on_create.contains("int priority() default 0;"),
        "expected generated Java @OnCreate annotation to be runtime-visible and model-scoped:\n{on_create}"
    );
    assert!(
        context.contains("public final class CrudEventContext")
            && context.contains("private final Map<String, Object> args;")
            && context.contains("private final String transactionId;")
            && context.contains("private final Map<String, Object> state;"),
        "expected generated Java event context to expose args, transaction id, and shared state:\n{context}"
    );
    assert!(
        stop.contains("public final class StopPropagation extends RuntimeException")
            && stop.contains("public Object result()"),
        "expected generated Java StopPropagation runtime type:\n{stop}"
    );
    assert!(
        options.contains("public NautilusOptions eventPackages(String... packageNames)")
            && options.contains("public List<String> eventPackages()")
            && options.contains("Collections.unmodifiableList(this.eventPackages)"),
        "expected NautilusOptions to expose opt-in event package scanning:\n{options}"
    );
    assert!(
        nautilus.contains("eventRegistry().registerAnnotatedPackages(options().eventPackages().toArray(String[]::new));"),
        "expected Nautilus client construction to register configured event packages:\n{nautilus}"
    );
    assert!(
        delegate.contains("return writeOne(")
            && delegate.contains("return writeMany(\"update\", \"query.update\",")
            && delegate.contains("return writeCount(\"deleteMany\", \"query.deleteMany\","),
        "expected Java delegate mutations to name their operation and wire method:\n{delegate}"
    );
    assert!(
        base_delegate
            .contains("eventContext(operation, EventPhase.BEFORE, eventArgs, request, state, null, null)")
            && base_delegate
                .contains("eventContext(operation, EventPhase.AFTER, eventArgs, request, state, decoded, null)")
            && base_delegate.contains(
                "eventContext(operation, EventPhase.ERROR, eventArgs, request, state, null, error), false"
            )
            && base_delegate.matches("EventPhase.BEFORE").count() == 3,
        "expected every Java write shape to run before/after/error CRUD events:\n{base_delegate}"
    );
    assert!(
        registry.contains("public void registerAnnotatedPackages(String... packageNames)")
            && registry.contains("scanDirectory(classes, loader")
            && registry.contains("scanJar(classes, loader")
            && registry.contains("Modifier.isStatic(method.getModifiers())")
            && registry.contains("candidate.getConstructor()")
            && registry.contains("constructor.newInstance()")
            && registry.contains("normalizePriority(")
            && registry.contains("registered.sort((left, right) -> Integer.compare(right.priority(), left.priority()))")
            && registry.contains("catch (StopPropagation stop)"),
        "expected Java EventRegistry to scan configured packages and invoke static/no-arg instance handlers:\n{registry}"
    );
}
