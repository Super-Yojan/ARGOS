#!/bin/bash
# The shared crate is not published; fetch the reviewed sibling source explicitly.
set -euo pipefail
revision=47b7694c9fbdddb0108ab6001382475f3bd93fd4
if [[ -e ../Terra ]]; then
    [[ $(git -C ../Terra rev-parse HEAD) == "$revision" ]] || { echo 'Existing sibling Terra differs from the reviewed observability source' >&2; exit 1; }
else
    git init ../Terra
    git -C ../Terra remote add origin https://github.com/Super-Yojan/Terra.git
    git -C ../Terra fetch --depth=1 origin "$revision"
    git -C ../Terra checkout --detach FETCH_HEAD
fi
