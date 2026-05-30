{-# OPTIONS --safe --without-K #-}

-- Orphan-rule fixture entry module. `Used.agda` is imported here;
-- `Orphan.agda` deliberately is NOT, so the `orphan-module` rule
-- must flag it as unreachable from the entry.

module All where

open import Used
