(import
  (scheme base)
  (scheme write)
  (neetan test 1))

(write
  (test-suite "detailed suite"
    (test-case "passing case"
      (check-equal 42 42))
    (test-case "assertion case"
      (fail "deliberate assertion")
      (note "unreachable assertion tail"))
    (test-case "error case"
      (error "deliberate error" 'neetan/test-error))
    (test-case "raised value case"
      (raise 7))
    (test-case "duplicate name"
      (check-true #t))
    (test-case "duplicate name"
      (check-false #f))))
(newline)
