import importlib.resources
import os
import sys


def main() -> None:
    binary_name = "nautilus.exe" if sys.platform == "win32" else "nautilus"
    binary = importlib.resources.files("nautilus") / binary_name
    os.execv(str(binary), [str(binary)] + sys.argv[1:])


if __name__ == "__main__":
    main()
