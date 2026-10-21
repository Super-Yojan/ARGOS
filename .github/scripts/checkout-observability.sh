#!/bin/bash
# The shared crate is not published; fetch the reviewed sibling source explicitly.
set -euo pipefail
revision=0fe32405595cbf65d0c4ad7a13f2c63b4c46ef05
if [[ -e ../Terra ]]; then
    [[ $(git -C ../Terra rev-parse HEAD) == "$revision" ]] || { echo 'Existing sibling Terra differs from the reviewed observability source' >&2; exit 1; }
else
    git init ../Terra
    git -C ../Terra remote add origin https://github.com/Super-Yojan/Terra.git
    git -C ../Terra fetch --depth=1 origin "$revision"
    git -C ../Terra checkout --detach FETCH_HEAD
fi
