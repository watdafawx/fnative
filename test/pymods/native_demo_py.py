"""called from Lua through the py plugin: native.call("py", "native_demo_py:hello", input)"""
import json
import sys
import threading

CALLS = 0  # module state survives between calls: the interpreter stays loaded


def hello(text: str) -> str:
    global CALLS
    CALLS += 1
    return json.dumps({"python": sys.version.split()[0], "got": text, "upper": text.upper(), "calls": CALLS,
                       "thread": threading.current_thread().name})


def fail(text: str) -> str:
    raise ValueError("a Python error, on purpose: " + text)
