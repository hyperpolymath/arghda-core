; A discharged verification condition: no integer is both > 2 and < 1.
; unsat  =>  the property holds  =>  Proven.
(set-logic QF_LIA)
(declare-const x Int)
(assert (and (> x 2) (< x 1)))
(check-sat)
