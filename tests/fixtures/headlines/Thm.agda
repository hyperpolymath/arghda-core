{-# OPTIONS --safe --without-K #-}

module Thm where

-- Two top-level headline signatures the DAG should surface, plus an
-- indented (non-top-level) helper that must NOT be surfaced.
thm-one : Set₁
thm-one = Set

thm-two : Set₁
thm-two = Set

private
  helper : Set₁
  helper = Set
