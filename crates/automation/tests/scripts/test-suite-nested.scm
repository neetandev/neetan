(import
  (scheme base)
  (neetan test 1))

(test-suite "outer suite"
  (test-suite "inner suite"
    (test-case "unreachable case"
      (check-true #t))))
