# Fixture: class inheritance and method override.
# Expected extraction: Class nodes + EXTENDS edges + METHOD_OVERRIDES edges.

from abc import ABC, abstractmethod

class Animal(ABC):
    @abstractmethod
    def speak(self) -> str:
        pass

class Dog(Animal):
    def speak(self) -> str:
        return "Woof!"
