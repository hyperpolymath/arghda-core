{-# OPTIONS --safe --without-K #-}
module Main where

-- Imports Helper but uses nothing from it: a genuine unused import, which is
-- exactly what `agda-unused` reports (re-emitted as `unused-import`).
open import Helper
