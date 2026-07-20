(import
  (scheme base)
  (neetan test 1))

(test-suite "invalid case name"
  (test-case 'invalid
    (check-true #t)))
