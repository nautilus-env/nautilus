"""Custom descriptors for Nautilus models."""

from typing import Any, Callable, Generic, Optional, TypeVar, overload


T = TypeVar("T")


class ClassPropertyDescriptor(Generic[T]):
    """Descriptor that allows property access on a class (not just instance).
    
    This enables the pattern: User.nautilus instead of User().nautilus
    """

    def __init__(self, fget: Callable[[type], T]) -> None:
        """Initialize the descriptor.
        
        Args:
            fget: The getter function that receives the class as argument.
        """
        self.fget = fget
        self.__doc__ = fget.__doc__

    @overload
    def __get__(self, obj: None, klass: type) -> T: ...
    
    @overload
    def __get__(self, obj: Any, klass: Optional[type] = None) -> T: ...

    def __get__(self, obj: Optional[Any], klass: Optional[type] = None) -> T:
        """Get the property value.
        
        Args:
            obj: The instance (will be None when accessed on class).
            klass: The class.
            
        Returns:
            The result of calling fget with the class.
        """
        if klass is None:
            klass = type(obj)
        return self.fget(klass)


def classproperty(func: Callable[[type], T]) -> ClassPropertyDescriptor[T]:
    """Decorator to create a class property.
    
    Usage:
        @classproperty
        def nautilus(cls) -> Delegate:
            return get_delegate_for_class(cls)
    
    Args:
        func: The getter function.
        
    Returns:
        A ClassPropertyDescriptor instance.
    """
    return ClassPropertyDescriptor(func)
