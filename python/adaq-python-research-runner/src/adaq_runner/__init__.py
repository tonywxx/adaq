"""Private Runner package; never re-exported by the public ``adaq`` SDK."""

__all__ = ["run"]

def __getattr__(name: str):
    if name == "run":
        from .__main__ import run

        return run
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
