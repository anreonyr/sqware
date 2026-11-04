# Router client intake

Status: **verified client prerequisite**. The client now lives only in the
published Router Unit and is consumed from a full public revision. This slice
moves the client and its test projections; it does not move the Router server,
declaration, shared support or runtime. Earlier migration and acceptance documents
remain historical.

## Selected revisions

- Source base: `ac5b61d8880c499f8cee4c60929c06cc9f4820d8`.
- Router Unit provider (API + client):
  `aa3033a7bd25b5d4199a50a6c00538475b62ef4c`.
- Driver aggregate: `40d655c9f0a5efceeaa0f6fa2cf23a0cfc6f411e`
  (`units/router` = `aa3033a…`, `units/hub` = `daa23b2…`).
- Canonical Router API: `94fec223f3c5df6c99a49527a65d016d88de3af4`.
- Hub API `cccef…`, Hub client `daa…`, `driver-support` `8e6…`,
  `driver-cluster` `f202…` and the System pins remain unchanged.

## Ownership and canonical identity

Source keeps the Router server, declaration, image and integration tests. The
four original local client files are deleted; `router-client` is consumed from
`router.git` at the full `aa3033a…` revision and the local workspace member is
gone. `Cargo.lock` changes only to add the `router-client` Git source. The client
keeps its canonical `router-api` Git `94fec…` edge, so the Source direct pin and
the client transitive pin resolve to one `router-api` package identity. No new
Router server, declaration or shared Context/Device/Fail support is added.

## Recorded verification

Evidence: `source-candidate/proof.json`, `source-candidate/router-elf.json`,
`runtime/judge.json`, `provider-public/proof.json` and `driver-public/proof.json`.

- Canonical locked metadata passes online and offline (`raw_exit 0`).
- 19 host tests pass (`raw_exit 0`): router-handoff 4, device-api 3,
  interface-marks 5, system-shape 7.
- One real Source `prog-router` RISC-V **release** ELF: machine `243`, `ET_EXEC`,
  entry `0x1a544`, `sha256 1634b98d628beaa6f712a1b48f5f14492b65e277b2c28698faa89036549d8d13`,
  324328 bytes; unique `_start`, `__rustc::rust_begin_unwind` and
  `__rustc::__rust_alloc`; gzip roundtrip exact.
- Exactly two real accept scenes, SMP4 and SMP8: each `raw_exit 0`, root exit
  `0x0` with `accept: done`, one natural halt, frames `85→85` and `153→153`, no
  panic lines; judge passed.

## Preserved test projections

- `programs/tests/router-handoff` keeps its four selectors, fixtures and the
  Source server helper; only the client handoff projection reads
  `components/driver/units/router/client/src/handoff.rs`.
- `programs/tests/system-shape` keeps every existing assertion, adds the
  `selected-router-client` scanner root, reads the selected Unit manifest/source,
  and checks the canonical `router.git` `94fec…` pin with no local `path` edge.
- `device-api` 3 and `interface-marks` 5 are unchanged.

## Limits and pending

- No full historical 20-scene matrix, forced-doom, DMA/fence-specific suite,
  debug Source ELF or full-health rerun is claimed; those remain historical or
  pending broader work.
- Source Draft PR publication and final independent review are recorded separately by Root.
- Session old URL pins are kept despite the repository rename; URL identity
  migration is a separate serial queue.
