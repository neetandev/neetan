(import
  (scheme base)
  (scheme write)
  (neetan test 1))

(write
  (test-suite "error suite"
    (test-case "error case"
      (error "unexpected case error" 'neetan/test-error))
    (test-case "later case"
      (note "later error case ran")
      (check-true #t))))
(newline)
