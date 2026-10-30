# Proposed workflow files

The fleet can't push files under `.github/workflows/`, so workflow YAML that needs
adding or changing waits here until Kay copies it into `.github/workflows/` and
pushes. Keep these files thin: all CI logic belongs in `scripts/ci.sh` and
`scripts/ci/`.
