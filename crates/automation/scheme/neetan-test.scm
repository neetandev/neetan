;; The public (neetan test 1) library.
(define-library (neetan test 1)
  (export
    test-suite test-case
    check-true check-false check-equal check-near check-screen
    fail note artifact!)
  (import (scheme base) (scheme write) (neetan automation 1) (neetan internal 1)
          (neetan handles internal 1))
  (begin
    (define-record-type <test-suite-state>
      (%make-test-suite-state name test-count passed-count failures)
      %test-suite-state?
      (name %test-suite-name)
      (test-count %test-count %set-test-count!)
      (passed-count %passed-count %set-passed-count!)
      (failures %failures %set-failures!))

    (define current-test-suite (make-parameter #f))
    (define current-test-case (make-parameter #f))
    (define root-test-suite-seen #f)

    (define (%test-state message)
      (error message 'neetan/test-state))

    (define (%require-test-case who)
      (if (and (current-test-suite) (current-test-case))
          (if #f #f)
          (%test-state (string-append who ": requires an active test-case"))))

    (define (%assert-fail message)
      (%require-test-case "assertion")
      (error message 'neetan/assertion))

    (define (%written value)
      (let ((port (open-output-string)))
        (write value port)
        (get-output-string port)))

    (define (%check-fail message check-form)
      (%assert-fail
        (string-append message "; check: " (%written check-form))))

    ;; Returns whether a condition is a neetan/assertion failure.
    (define (%assertion? condition)
      (and (error-object? condition)
           (memq 'neetan/assertion (error-object-irritants condition))
           #t))

    (define (fail message)
      (%require-test-case "fail")
      (if (string? message)
          (%assert-fail message)
          (error "fail message must be a string" 'neetan/argument)))

    (define (note message)
      (if (string? message)
          (%emit-note message)
          (error "note message must be a string" 'neetan/argument)))

    (define (artifact! artifact-path)
      (if (string? artifact-path)
          (begin
            (%emit-note (string-append "artifact: " artifact-path))
            (list (cons 'path artifact-path)))
          (error "artifact! path must be a string" 'neetan/argument)))

    (define (%condition-message condition)
      (if (error-object? condition)
          (error-object-message condition)
          "test case raised a non-error condition"))

    (define (%record-failure suite name condition)
      (let ((failure
              (list
                (cons 'test-case name)
                (cons 'kind (if (%assertion? condition) 'assertion 'error))
                (cons 'message (%condition-message condition)))))
        (%set-failures! suite (cons failure (%failures suite)))
        failure))

    (define (%call-with-test-case name thunk)
      (cond
        ((not (string? name))
         (error "test-case name must be a string" 'neetan/argument))
        ((not (procedure? thunk))
         (error "test-case body must be a procedure" 'neetan/argument))
        ((not (current-test-suite))
         (%test-state "test-case requires an active test-suite"))
        ((current-test-case)
         (%test-state "test-case may not be nested"))
        (else
         (let ((suite (current-test-suite)))
           (%set-test-count! suite (+ (%test-count suite) 1))
           (parameterize ((current-test-case name))
             (guard (condition
                     (#t
                      (let ((failure (%record-failure suite name condition)))
                        (%emit-test-case-result
                          (%test-suite-name suite)
                          name
                          'failure
                          (alist-ref failure 'kind)
                          (alist-ref failure 'message)))
                      (if #f #f)))
               (thunk)
               (%set-passed-count! suite (+ (%passed-count suite) 1))
               (%emit-test-case-result
                 (%test-suite-name suite) name 'success 'success "")
               (if #f #f)))))))

    (define (%failure-line failure)
      (string-append
        (alist-ref failure 'test-case)
        ": "
        (alist-ref failure 'message)))

    (define (%failure-lines failures)
      (if (null? failures)
          ""
          (let loop ((rest failures) (result ""))
            (if (null? rest)
                result
                (loop (cdr rest)
                      (string-append result "\n" (%failure-line (car rest))))))))

    (define (%call-with-test-suite name thunk)
      (cond
        ((not (string? name))
         (error "test-suite name must be a string" 'neetan/argument))
        ((not (procedure? thunk))
         (error "test-suite body must be a procedure" 'neetan/argument))
        ((current-test-suite)
         (%test-state "test-suite may not be nested"))
        (root-test-suite-seen
         (%test-state "test-suite root may only appear once"))
        (else
         (set! root-test-suite-seen #t)
         (let ((suite (%make-test-suite-state name 0 0 '())))
           (parameterize ((current-test-suite suite))
             (thunk))
           (if (= (%test-count suite) 0)
               (%test-state "test-suite requires at least one test-case")
               (let* ((failures (reverse (%failures suite)))
                      (failure-count (length failures))
                      (passed (= failure-count 0))
                      (summary
                        (if passed
                            (string-append
                              name ": " (number->string (%test-count suite))
                              " test case(s) passed")
                            (string-append
                              name ": " (number->string failure-count)
                              " of " (number->string (%test-count suite))
                              " test case(s) failed"
                              (%failure-lines failures))))
                      (result
                        (list
                          (cons 'suite name)
                          (cons 'passed passed)
                          (cons 'test-count (%test-count suite))
                          (cons 'passed-count (%passed-count suite))
                          (cons 'failure-count failure-count)
                          (cons 'failures failures)
                          (cons 'summary summary))))
                 (if passed
                     (execution-result 'OK)
                     (execution-result 'ERROR summary))
                 result))))))

    (define-syntax test-suite
      (syntax-rules ()
        ((_ name body ...)
         (%call-with-test-suite name
           (lambda ()
             (if #f #f)
             body ...)))))

    (define-syntax test-case
      (syntax-rules ()
        ((_ name body ...)
         (%call-with-test-case name
           (lambda ()
             (if #f #f)
             body ...)))))

    (define (%check-true check-form value)
      (%require-test-case "check-true")
      (if value
          value
          (%check-fail "check-true failed: value was false" check-form)))

    (define (%check-false check-form value)
      (%require-test-case "check-false")
      (if value
          (%check-fail "check-false failed: value was true" check-form)
          value))

    (define (%check-equal check-form expected actual)
      (%require-test-case "check-equal")
      (if (equal? expected actual)
          actual
          (%check-fail "check-equal failed: values are not equal" check-form)))

    (define (%check-near check-form expected actual tolerance)
      (%require-test-case "check-near")
      (if (not (and (real? expected) (real? actual)
                    (real? tolerance) (>= tolerance 0)))
          (error "check-near expects reals and a non-negative tolerance"
                 'neetan/argument)
          (if (<= (abs (- expected actual)) tolerance)
              actual
              (%check-fail
                (string-append "check-near failed: |"
                               (number->string expected) " - "
                               (number->string actual) "| > "
                               (number->string tolerance))
                check-form))))

    ;; Best-effort extraction of a written comparison-image path for a message.
    (define (%comparison-path result)
      (if (and (pair? result) (eq? (car result) '%error))
          "(comparison image unavailable)"
          (alist-ref result 'path)))

    (define (%check-screen check-form machine expected-path . optional-options)
      (%require-test-case "check-screen")
      (if (not (string? expected-path))
          (error "check-screen path must be a string" 'neetan/argument)
          (let ((matches
                  (cond
                    ((null? optional-options)
                     (screen-matches? machine expected-path))
                    ((null? (cdr optional-options))
                     (screen-matches?
                       machine expected-path (car optional-options)))
                    (else
                      (error "check-screen: expected two or three arguments"
                             'neetan/argument)))))
            (if matches
                (if #f #f)
                (let ((artifact
                        (%screen-comparison-image
                          (%require-machine-token "check-screen" machine)
                          expected-path)))
                  (%check-fail
                    (string-append "check-screen failed: " expected-path
                                   " did not match; comparison image at "
                                   (%comparison-path artifact))
                    check-form))))))

    (define-syntax check-true
      (syntax-rules ()
        ((_ arguments ...)
         (%check-true '(check-true arguments ...) arguments ...))))

    (define-syntax check-false
      (syntax-rules ()
        ((_ arguments ...)
         (%check-false '(check-false arguments ...) arguments ...))))

    (define-syntax check-equal
      (syntax-rules ()
        ((_ arguments ...)
         (%check-equal '(check-equal arguments ...) arguments ...))))

    (define-syntax check-near
      (syntax-rules ()
        ((_ arguments ...)
         (%check-near '(check-near arguments ...) arguments ...))))

    (define-syntax check-screen
      (syntax-rules ()
        ((_ arguments ...)
         (%check-screen '(check-screen arguments ...) arguments ...))))

    ))
