;; Optional input settings use the shared options association-list contract.
(import (scheme base) (neetan automation 1) (neetan test 1))

(define (raises? symbol thunk)
  (guard (condition
          ((and (error-object? condition)
                (memq symbol (error-object-irritants condition)))
           #t)
          (else #f))
    (thunk)
    #f))

(define (check-frame-advance machine expected thunk)
  (let ((frame (machine-frame machine)))
    (thunk)
    (check-equal (+ frame expected) (machine-frame machine))))

(test-suite "Input options"
  (with-machine (machine '((model . pc9801vm)))
    (test-case "defaults hold the key for two frames"
      (check-frame-advance machine 2
        (lambda () (key-tap! machine 'a))))

    (test-case "an empty options list uses the defaults"
      (check-frame-advance machine 2
        (lambda () (key-tap! machine 'a '()))))

    (test-case "frames and ticks can be overridden independently"
      (check-frame-advance machine 1
        (lambda () (key-tap! machine 'a '((frames . 1)))))
      (check-frame-advance machine 2
        (lambda () (key-tap! machine 'a '((ticks . 100000000)))))
      (check-frame-advance machine 1
        (lambda ()
          (key-tap! machine 'a
            '((frames . 1) (ticks . 50000000))))))

    (test-case "zero frames derives a zero tick limit"
      (check-frame-advance machine 0
        (lambda () (key-tap! machine 'a '((frames . 0))))))

    (test-case "type-text! defaults to two frames per character"
      (check-frame-advance machine 2
        (lambda () (type-text! machine "A"))))

    (test-case "type-text! options can be overridden independently"
      (check-frame-advance machine 2
        (lambda () (type-text! machine "F" '())))
      (check-frame-advance machine 1
        (lambda () (type-text! machine "B" '((frames . 1)))))
      (check-frame-advance machine 2
        (lambda () (type-text! machine "C" '((ticks . 100000000)))))
      (check-frame-advance machine 1
        (lambda ()
          (type-text! machine "D"
            '((frames . 1) (ticks . 50000000)))))
      (check-frame-advance machine 0
        (lambda () (type-text! machine "E" '((frames . 0))))))

    (test-case "option validation uses the argument contract"
      (check-true
        (raises? 'neetan/argument
          (lambda () (key-tap! machine 'a #f))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (key-tap! machine 'a '((unknown . 1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (key-tap! machine 'a '((frames . 1) (frames . 2))))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (key-tap! machine 'a '((frames . -1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (key-tap! machine 'a '(frames . 1)))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (key-tap! machine 'a 2 5000000)))))

    (test-case "type-text! option validation uses the argument contract"
      (check-true
        (raises? 'neetan/argument
          (lambda () (type-text! machine "A" #f))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (type-text! machine "A" '((unknown . 1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (type-text! machine "A" '((frames . 1) (frames . 2))))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (type-text! machine "A" '((ticks . -1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (type-text! machine "A" '(frames . 1)))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (type-text! machine "A" 2 5000000)))))))
