# tomboy extensions

This folder is tomboy's extension registry. Each subfolder is one extension; `index.json`
lists them all and is what the store inside tomboy reads (from
`raw.githubusercontent.com/dzhibas/tomboy/main/extensions/`). Every file here was reviewed
in a pull request before users could install it.

| Extension | What it does |
| --- | --- |
| [`hackernews`](hackernews) | Hacker News front page, newest, Show HN and Ask HN; the sample to copy from |

## Add an extension

1. Fork tomboy and clone your fork.
2. Create the folder: `cd extensions && tomboy ext new my-ext`.
3. Develop it: `tomboy ext dev extensions/my-ext` (from the repository root), Ctrl+R (⌘R) to
   reload after each save.
4. Write `README.md`: what it does, its keys and actions, its preferences.
5. Check it and regenerate the index, from the repository root:

   ```sh
   tomboy ext check extensions/my-ext
   tomboy ext index
   ```

6. Commit the folder and `index.json` together and open a pull request. One extension per
   pull request, and nothing outside `extensions/` in it.

To change an existing extension, bump its `version` in `tomboy.toml`, then do steps 5 and 6.

- How to write one, and the whole API: [docs/extensions.md](../docs/extensions.md)
- The review process and checklist: [CONTRIBUTING.md](../CONTRIBUTING.md#extensions)
