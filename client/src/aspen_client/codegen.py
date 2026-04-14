from __future__ import annotations

import subprocess
import sys
from pathlib import Path


def main() -> int:
    client_root = Path(__file__).resolve().parents[2]
    repo_root = client_root.parent
    generated_dir = client_root / "src" / "aspen_client" / "generated"
    generated_dir.mkdir(parents=True, exist_ok=True)

    openapi_file = repo_root / "openapi.yaml"
    event_schema_file = repo_root / "event_schema.json"
    openapi_out = generated_dir / "openapi_models.py"
    event_out = generated_dir / "event_models.py"

    run_codegen(
        [
            sys.executable,
            "-m",
            "datamodel_code_generator",
            "--input",
            str(openapi_file),
            "--input-file-type",
            "openapi",
            "--output",
            str(openapi_out),
            "--target-python-version",
            "3.12",
            "--use-standard-collections",
            "--use-subclass-enum",
            "--field-constraints",
            "--enum-field-as-literal",
            "one",
        ]
    )
    run_codegen(
        [
            sys.executable,
            "-m",
            "datamodel_code_generator",
            "--input",
            str(event_schema_file),
            "--input-file-type",
            "jsonschema",
            "--output",
            str(event_out),
            "--target-python-version",
            "3.12",
            "--use-standard-collections",
            "--use-subclass-enum",
            "--field-constraints",
            "--enum-field-as-literal",
            "one",
        ]
    )
    print(f"Generated {openapi_out} and {event_out}")
    return 0


def run_codegen(command: list[str]) -> None:
    result = subprocess.run(command, check=False, capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(
            "Code generation failed:\n"
            f"COMMAND: {' '.join(command)}\n"
            f"STDOUT:\n{result.stdout}\n"
            f"STDERR:\n{result.stderr}"
        )


if __name__ == "__main__":
    raise SystemExit(main())
