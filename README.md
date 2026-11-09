# Vidcue release mirror

This repository only mirrors the releases of the canonical repository
[Vanir20342222/vidcue](https://github.com/Vanir20342222/vidcue) so that older
installations and links that use the previous `ytui` repository name keep working.

The canonical source code, issues and pull requests live at
[Vanir20342222/vidcue](https://github.com/Vanir20342222/vidcue).

A scheduled workflow syncs every non-draft release (tags, notes and assets) from
the canonical repository. It can also be run manually from the Actions tab
(**Mirror releases from vidcue**). Releases that already exist here are skipped;
their notes are updated when the source notes change.

If no releases occur for 60 days, GitHub may disable the scheduled workflow;
re-enable it from the Actions tab.
