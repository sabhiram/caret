# Vendored editor assets

`milkdown.bundle.js` / `milkdown.bundle.css` are a self-contained build of the
[Milkdown Crepe](https://milkdown.dev) editor, inlined into served pages so the
rendered HTML has **zero network dependencies**.

## Rebuild recipe

```sh
mkdir build && cd build && npm init -y
npm i @milkdown/crepe esbuild
# entry.js: import Crepe + theme css, expose window.DocdEditor.mount(root, md, onChange)
npx esbuild entry.js --bundle --format=iife --minify \
  --loader:.woff=dataurl --loader:.woff2=dataurl --loader:.ttf=dataurl \
  --loader:.svg=dataurl --loader:.png=dataurl --loader:.gif=dataurl \
  --outfile=milkdown.bundle.js
# Strip KaTeX @font-face blocks (LaTeX is disabled; ~1.4MB of unused fonts):
#   python: re.sub(r'@font-face\{[^}]*KaTeX[^}]*\}', '', css)
```

The page must stay free of external URLs — verify with:
`grep -E 'url\((["'\'']?)(https?:)?//' milkdown.bundle.css` (expect no matches).
