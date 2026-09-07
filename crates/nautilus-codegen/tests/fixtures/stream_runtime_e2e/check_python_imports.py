import importlib
import inspect


for model in ("Example", "Parent", "Summary"):
    source = importlib.import_module(f"source_client.models.{model.lower()}")
    modular = importlib.import_module(f"modular_client.models.{model.lower()}")
    source_symbols = {name for name in vars(source) if not name.startswith("_")}
    modular_symbols = {name for name in vars(modular) if not name.startswith("_")}
    assert source_symbols == modular_symbols, (
        model, source_symbols - modular_symbols, modular_symbols - source_symbols
    )
    assert source.__all__ == modular.__all__, model
    assert getattr(source, model).__annotations__ == getattr(modular, model).__annotations__, model
    source_delegate = getattr(source, f"{model}Delegate")
    modular_delegate = getattr(modular, f"{model}Delegate")
    methods = {name for name in dir(source_delegate) if not name.startswith("_")}
    assert methods == {name for name in dir(modular_delegate) if not name.startswith("_")}, model
    for method in methods | {"__init__"}:
        source_method = getattr(source_delegate, method)
        modular_method = getattr(modular_delegate, method)
        assert str(inspect.signature(source_method)) == str(inspect.signature(modular_method)), (model, method)
        assert inspect.iscoroutinefunction(source_method) == inspect.iscoroutinefunction(modular_method), (model, method)

print("Python public imports and delegate signatures match")
