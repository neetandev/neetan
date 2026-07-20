(import
  (scheme base)
  (neetan test 1))

(test-suite "first suite"
  (test-case "first case"
    (check-true #t)))

(test-suite "second suite"
  (test-case "must not run"
    (note "second root case ran")))
