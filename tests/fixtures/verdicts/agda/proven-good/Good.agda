{-# OPTIONS --safe --without-K #-}
module Good where
data N : Set where
  z : N
  s : N -> N
