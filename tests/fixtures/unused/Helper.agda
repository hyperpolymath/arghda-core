module Helper where

-- A trivial, dependency-free definition. Nothing references it, so a
-- whole-project (`--global`) agda-unused pass should also flag it.
helper : Set₁
helper = Set
