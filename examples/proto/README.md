# examples/proto — prototypes and throwaway books

Everything under this directory is **excluded from the crate package** as a
whole (`exclude = ["examples/proto/"]` in `Cargo.toml`), so prototypes,
half-built books, and ICE-derived market data can live here without
per-directory bookkeeping. `just pkg-check` (part of `just ci`) fails if
`cargo package` would ship any example besides `example-public` and
`bond_pricing`.

Seed a prototype from an existing example:

    just proto-init <name>                # copy of brent-condor
    just proto-init <name> brent-option   # copy of another example
    cd examples/proto/<name> && just      # book / day / series / risk / scenarios

Whether a prototype is committed to git is a separate decision from the
crate boundary — `git add` it or leave it untracked.
