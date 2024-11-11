# Beer Thirty Tap Menu

This is a rust-based Cloudflare worker for reconstituting the tap menu at [Beer Thirty](https://www.beerthirtysantacruz.com/) (a popular tap room in Santa Cruz, CA).

In particular, this worker:

1. Fetches and parses the [bthirty.com](http://bthirty.com) TapHunter menu.
2. Cross-references [Untapp'd](https://untappd.com) for ratings + review links.
3. Groups by category (IPAs, Sours, etc).
4. Sorts within the group by ABV and heatmaps the ABV column.
5. Renders this in a table for easy tap selection.

This worker is compiled to a wasm binary, which is then composed with some shims by the `worker-build` binary to produce an E2E working rust Cloudflare worker.

# Development

## Run a Local Dev Server

```
$ npx wrangler dev
```

## Dry-run Deploy to Cloudflare

This is useful for checking the final package size before deploying, which must be <1MB for free Cloudflare deployments.

```
$ npx wrangler deploy --dry-run
```

## Deploy to Cloudflare

```
$ npx wrangler deploy
```
