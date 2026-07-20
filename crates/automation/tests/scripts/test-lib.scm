(import
  (scheme base)
  (neetan test 1))

(test-suite "test library"
  (test-case "passes a successful case"
    (check-true #t)))
