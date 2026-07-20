(import
  (scheme base)
  (neetan test 1))

(test-suite "nested case suite"
  (test-case "outer case"
    (test-case "inner case"
      (check-true #t))))
