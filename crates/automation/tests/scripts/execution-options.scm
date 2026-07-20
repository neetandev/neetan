;; Optional execution bounds use association lists and derived tick defaults.
(import (scheme base) (neetan automation 1) (neetan test 1))

(define (raises? symbol thunk)
  (guard (condition
          ((and (error-object? condition)
                (memq symbol (error-object-irritants condition)))
           #t)
          (else #f))
    (thunk)
    #f))

(test-suite "Execution options"
  (with-machine (machine '((model . pc9801vm)))
    (test-case "run-until-frame! derives its tick limit"
      (let ((target (+ (machine-frame machine) 2)))
        (run-until-frame! machine target)
        (check-equal target (machine-frame machine)))
      (let ((frame (machine-frame machine)))
        (run-until-frame! machine frame)
        (check-equal frame (machine-frame machine))))

    (test-case "run-until-frame! accepts an explicit tick limit"
      (let ((target (+ (machine-frame machine) 1)))
        (run-until-frame! machine target '((ticks . 50000000)))
        (check-equal target (machine-frame machine)))
      (let ((target (+ (machine-frame machine) 1)))
        (run-until-frame! machine target '())
        (check-equal target (machine-frame machine))))

    (test-case "wait-until uses defaults and explicit bounds"
      (let ((target (+ (machine-frame machine) 2)))
        (check-true
          (wait-until machine
            (lambda () (>= (machine-frame machine) target))))
        (check-equal target (machine-frame machine)))
      (let ((frame (machine-frame machine)))
        (check-false
          (wait-until machine (lambda () #f) '((frames . 0))))
        (check-equal frame (machine-frame machine)))
      (check-true
        (wait-until machine (lambda () #t) '()))
      (check-true
        (wait-until machine (lambda () #t) '((ticks . 0))))
      (check-false
        (wait-until machine (lambda () #f)
          '((frames . 0) (ticks . 0)))))

    (test-case "run-until-frame! options use the argument contract"
      (check-true
        (raises? 'neetan/argument
          (lambda () (run-until-frame! machine 0 #f))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (run-until-frame! machine 0 '((unknown . 1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (run-until-frame! machine 0 '((ticks . 1) (ticks . 2))))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (run-until-frame! machine 0 '((ticks . -1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (run-until-frame! machine 0 0))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (run-until-frame! machine 0 '() '())))))

    (test-case "wait-until options use the argument contract"
      (check-true
        (raises? 'neetan/argument
          (lambda () (wait-until machine (lambda () #t) #f))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (wait-until machine (lambda () #t) '((unknown . 1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (wait-until machine (lambda () #t)
              '((frames . 1) (frames . 2))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (wait-until machine (lambda () #t) '((frames . -1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (wait-until machine (lambda () #t) 1 50000000)))))))
