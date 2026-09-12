#!/usr/bin/env bash
set -euo pipefail

test_root=$(mktemp -d)
trap 'rm -rf -- "$test_root"' EXIT

cat > "$test_root/method.rs" <<'EOF'
fn main() {
    client
        .perform_required_write();
}
EOF
cat > "$test_root/method.info" <<EOF
SF:$test_root/method.rs
DA:1,1
DA:2,1
DA:3,0
DA:4,1
end_of_record
EOF

if bash scripts/check_coverage.sh "$test_root/method.info" >/dev/null 2>&1; then
  echo "coverage check accepted an uncovered method call" >&2
  exit 1
fi

cat > "$test_root/field.rs" <<'EOF'
fn main() {
    let state = State {
        required: perform_required_write(),
    };
}
EOF
cat > "$test_root/field.info" <<EOF
SF:$test_root/field.rs
DA:1,1
DA:2,1
DA:3,0
DA:4,1
DA:5,1
end_of_record
EOF

if bash scripts/check_coverage.sh "$test_root/field.info" >/dev/null 2>&1; then
  echo "coverage check accepted an uncovered field expression" >&2
  exit 1
fi

cat > "$test_root/one-line.rs" <<'EOF'
fn perform() { perform_required_write(); }
EOF
cat > "$test_root/one-line.info" <<EOF
SF:$test_root/one-line.rs
DA:1,0
end_of_record
EOF

if bash scripts/check_coverage.sh "$test_root/one-line.info" >/dev/null 2>&1; then
  echo "coverage check accepted an uncovered one-line function body" >&2
  exit 1
fi

cat > "$test_root/continuation.rs" <<'EOF'
fn main() {
    let value = Some(
        1
    ).unwrap_or_else(|| {
        perform_required_write()
    });
}
EOF
cat > "$test_root/continuation.info" <<EOF
SF:$test_root/continuation.rs
DA:1,1
DA:2,1
DA:3,1
DA:4,0
DA:5,1
DA:6,1
DA:7,1
end_of_record
EOF

if bash scripts/check_coverage.sh "$test_root/continuation.info" >/dev/null 2>&1; then
  echo "coverage check accepted an uncovered call continuation" >&2
  exit 1
fi

cat > "$test_root/structure.rs" <<'EOF'
#[derive(Debug)]
fn main(
) {
}
EOF
cat > "$test_root/structure.info" <<EOF
SF:$test_root/structure.rs
DA:1,0
DA:2,0
DA:3,0
DA:4,0
end_of_record
EOF

bash scripts/check_coverage.sh "$test_root/structure.info" >/dev/null
