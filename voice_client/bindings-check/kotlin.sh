#!/usr/bin/env bash
# Builds the engine for this machine, writes its Kotlin bindings, compiles them with `Check.kt`,
# and runs the check against the built library. Needs `kotlinc` (on the PATH, or named by
# `KOTLINC`) and a JDK (`JAVA_HOME`, or `java` on the PATH); JNA and kotlinx-coroutines are
# fetched from Maven Central once into `target/bindings-check/jars/`.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
out="$root/target/bindings-check"
jars="$out/jars"
kotlinc="${KOTLINC:-kotlinc}"
java="${JAVA_HOME:+$JAVA_HOME/bin/}java"

mkdir -p "$jars"
fetch() {
  [ -f "$jars/$1" ] || curl -sSfL -o "$jars/$1" "https://repo1.maven.org/maven2/$2"
}
fetch jna-5.17.0.jar net/java/dev/jna/jna/5.17.0/jna-5.17.0.jar
fetch kotlinx-coroutines-core-jvm-1.10.2.jar \
  org/jetbrains/kotlinx/kotlinx-coroutines-core-jvm/1.10.2/kotlinx-coroutines-core-jvm-1.10.2.jar
classpath="$jars/jna-5.17.0.jar:$jars/kotlinx-coroutines-core-jvm-1.10.2.jar"

rm -rf "$out/kotlin"
python3 "$root/scripts/voice_client_bindings.py" --language kotlin --kotlin-dir "$out/kotlin"
"$kotlinc" -cp "$classpath" -include-runtime -d "$out/check.jar" \
  "$out/kotlin/com/aspen_chat/client/voice/voice_client.kt" "$root/voice_client/bindings-check/Check.kt"
"$java" -cp "$out/check.jar:$classpath" -Djna.library.path="$root/target/debug" CheckKt
