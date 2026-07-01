; A satisfiable query: a model (counterexample) exists.
; sat  =>  Refuted.
(set-logic QF_LIA)
(declare-const x Int)
(assert (> x 2))
(check-sat)
