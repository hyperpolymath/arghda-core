{-# OPTIONS --safe --without-K #-}

-- Missing-pragma fixture entry module. `Bad.agda` lacks the safe
-- pragma; the `missing-safe-pragma` rule must flag it.

module All where

open import Bad
