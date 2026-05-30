{-# OPTIONS --safe --without-K #-}

-- Well-formed fixture entry module. Used by arghda-core's smoke test
-- (tests/smoke.rs::wellformed_passes_both_default_rules) to confirm
-- a minimal Agda workspace passes both `missing-safe-pragma` and
-- `orphan-module` rules.

module All where

open import Good
open import Util
