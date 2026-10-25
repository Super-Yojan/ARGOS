#!/bin/bash
# The shared crate is not published; fetch the reviewed sibling source explicitly.
set -euo pipefail
revision=0df760ef4af4c382e9c82fef5c554c582b6ede96
if [[ -e ../Terra ]]; then
    [[ $(git -C ../Terra rev-parse HEAD) == "$revision" ]] || { echo 'Existing sibling Terra differs from the reviewed observability source' >&2; exit 1; }
else
    git init ../Terra
    git -C ../Terra remote add origin https://github.com/Super-Yojan/Terra.git
    git -C ../Terra fetch --depth=1 origin "$revision"
    git -C ../Terra checkout --detach FETCH_HEAD
fi
