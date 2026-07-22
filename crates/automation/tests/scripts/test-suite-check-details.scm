(import
  (scheme base)
  (scheme write)
  (neetan test 1))

(define evaluations 0)

(write
  (test-suite "check detail suite"
    (test-case "check-true detail"
      (check-true
        (begin
          (set! evaluations (+ evaluations 1))
          #f)))
    (test-case "check-false detail"
      (check-false (> 2 1)))
    (test-case "check-equal detail"
      (check-equal (+ 1 1) 3))
    (test-case "check-near detail"
      (check-near 1.0 2.0 0.1))
    (test-case "arguments are evaluated once"
      (check-equal 1 evaluations))))
(newline)
