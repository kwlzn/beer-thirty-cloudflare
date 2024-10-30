# Beer Thirty Tap Menu

This is a rust-based Cloudflare worker for reconstituting the tap menu at [Beer Thirty](https://www.beerthirtysantacruz.com/) (a popular tap room in Santa Cruz, CA).

In particular, this worker:

1. Fetches and parses the [bthirty.com](http://bthirty.com) TapHunter menu.
2. Cross-references [Untapp'd](https://untappd.com) for ratings + review links.
3. Groups by category (IPAs, Sours, etc).
4. Sorts within the group by ABV and heatmaps the ABV column.
5. Renders this in a table for easy tap selection.

