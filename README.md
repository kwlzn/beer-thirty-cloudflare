# Beer Thirty Tap Menu

This is a [rust-based Cloudflare worker](https://developers.cloudflare.com/workers/languages/rust/) for enriching and reconstituting the tap menu at [Beer Thirty](https://www.beerthirtysantacruz.com/) (a popular tap room in Santa Cruz, CA).

In particular, this worker:

1. Fetches and parses the [bthirty.com](http://bthirty.com) TapHunter menu.
2. Cross-references [Untappd](https://untappd.com) for ratings + review links.
3. Groups by category (IPAs, Sours, etc).
4. Sorts within the group by ABV and heatmaps the ABV column.
5. Renders this in a table for easy tap selection.

This code is compiled to a wasm binary, which is then composed with some shims by the `worker-build` binary to produce an E2E working rust Cloudflare worker.

## Architecture

All parsing/rendering lives in **pure modules** with no `worker` dependency, so they
compile and unit-test on the host (`cargo test`). The Cloudflare-specific glue (HTTP +
KV cache + the fetch event) is gated to the wasm target.

| Module            | Responsibility                                                    |
|-------------------|-------------------------------------------------------------------|
| `b30/taphunter.rs`| Resolve + parse the TapHunter menu JSON.                          |
| `b30/untappd.rs`  | Look up ratings via Untappd's Algolia search API (see below).     |
| `b30/render.rs`   | Render the sorted, rated taps to the HTML table.                  |
| `b30/model.rs`    | `BeerEntry`, `RatingResult`, sorting.                             |
| `b30/error.rs`    | Shared `AppError` / `AppResult`.                                  |
| `b30/lib.rs`      | wasm-only worker: fetch wrappers, KV cache, `#[event(fetch)]`.    |
| `b30/bin/dev.rs`  | Native dev runner (full pipeline against live sites, no deploy).  |

# Development

## Test (offline, deterministic)

Pure parser/renderer tests run against saved fixtures in `b30/fixtures/` — no network:

```
$ cargo test
```

## Run the full pipeline locally (live, no deploy)

The native dev runner reuses the exact same parsers/renderer the worker uses, but fetches
over plain HTTP — handy for iterating from a residential IP without deploying:

```
$ cargo run --features native --bin b30-dev -- rating "Sierra Nevada Pale Ale"
$ cargo run --features native --bin b30-dev -- menu menu.html   # open menu.html in a browser
$ cargo run --features native --bin b30-dev -- refresh-fixtures # re-capture b30/fixtures/
```

## Run the Worker locally

```
$ npx wrangler dev
```

## Dry-run Deploy to Cloudflare

Useful for checking the final package size before deploying, which must be <1MB
(compressed) for free Cloudflare deployments.

```
$ npx wrangler deploy --dry-run
```

## Deploy to Cloudflare

```
$ npx wrangler deploy
```

> The wasm build enables `reference-types` via `.cargo/config.toml` (required by the
> `worker-build` / `wasm-bindgen` packaging step).

## Caching

Ratings are cached in Workers KV. Cache keys are versioned (`rating:v2:…`); bump
`CACHE_VERSION` in `b30/lib.rs` to invalidate every cached entry at once (e.g. after a
parser change). Successful ratings cache for a week and confirmed "not found" for a day;
transient failures (network/blocked) are never cached, so they self-heal. To purge old
entries manually instead: `npx wrangler kv key list` / `delete` against the `b30` namespace.
