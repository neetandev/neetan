(import
  (scheme base)
  (scheme write)
  (neetan test 1))

(write
  (test-suite "failure suite"
    (test-case "assertion case"
      (check-true #f))
    (test-case "later case"
      (note "later case ran")
      (check-true #t))))
(newline)
