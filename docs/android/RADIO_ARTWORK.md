# Radio artwork

Provider-owned image/artwork/cover/thumbnail/logo/favicon URLs take precedence.
When a station has no valid HTTP(S) artwork URL, a curated genre photograph is
used. These photographs illustrate listening genres, without implying they show
the station or a currently playing artist. They were fetched and visually
reviewed on 2026-10-28; no generated or third-party images are bundled in Git.

Fallback image sources (Unsplash contributors, [Unsplash License](https://unsplash.com/license)):

- [Studio instruments](https://images.unsplash.com/photo-1511379938547-c1f69419868d): general / lofi.
- [Alpine lake](https://images.unsplash.com/photo-1470770841072-f978cf4d019e): ambient / meditation.
- [Live music](https://images.unsplash.com/photo-1493225457124-a3eb161ffa5f): rock / pop / electronic.
- [Saxophone](https://images.unsplash.com/photo-1415201364774-f6f0bb35f28f): jazz / soul / blues.

The shared image cache chooses 128/256/512/768-pixel square variants. Failed
artwork requests never block radio playback. Provider artwork remains subject to
its owner's terms; fallback photographs use the Unsplash license linked above.
