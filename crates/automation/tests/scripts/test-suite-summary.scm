(import
  (scheme base)
  (scheme write)
  (neetan test 1))

(write
  (test-suite "summary suite"
    (test-case "passing case"
      (check-equal 42 42))))
(newline)
