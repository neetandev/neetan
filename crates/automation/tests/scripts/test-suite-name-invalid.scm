(import
  (scheme base)
  (neetan test 1))

(test-suite 'invalid
  (test-case "case"
    (check-true #t)))
