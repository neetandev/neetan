(import (scheme base)
        (neetan automation 1)
        (neetan test 1))

(define (raises? symbol thunk)
  (guard (condition
          ((and (error-object? condition)
                (memq symbol (error-object-irritants condition)))
           #t)
          (else #f))
    (thunk)
    #f))

(test-suite "wait-for-screen"
  (with-machine (machine '((model . PC9801VM)))
    (test-case "default options reach a screen"
      (check-true
        (wait-for-screen machine "expected/pc98-title.png")))

    (test-case "screen comparisons default to exact matching"
      (check-true
        (screen-matches? machine "expected/pc98-title.png"))
      (check-true
        (screen-matches? machine "expected/pc98-title.png" '()))
      (check-true
        (screen-matches? machine "expected/pc98-wrong.png"
          '((tolerance . 1.0))))
      (check-true
        (screen-region-matches?
          machine "expected/pc98-title.png" 0 0 640 400))
      (check-true
        (screen-region-matches?
          machine "expected/pc98-title.png" 0 0 640 400 '()))
      (check-true
        (screen-region-matches?
          machine "expected/pc98-wrong.png" 0 0 640 400
          '((tolerance . 1.0))))
      (check-screen machine "expected/pc98-title.png")
      (check-screen machine "expected/pc98-title.png" '())
      (check-screen machine "expected/pc98-wrong.png"
        '((tolerance . 1.0))))

    (test-case "an immediate match consumes no frames"
      (let ((frame (machine-frame machine)))
        (check-true
          (wait-for-screen machine "expected/pc98-title.png"
            '((frames . 0) (ticks . 0))))
        (check-equal frame (machine-frame machine))))

    (test-case "explicit options are accepted"
      (restore-startup! machine)
      (check-true
        (wait-for-screen machine "expected/pc98-title.png"
          '((tolerance . 1.0) (frames . 1) (ticks . 50000000)))))

    (test-case "an exhausted wait returns false"
      (check-false
        (wait-for-screen machine "expected/pc98-wrong.png"
          '((frames . 0) (ticks . 0)))))

    (test-case "screen comparison options use the argument contract"
      (check-true
        (raises? 'neetan/argument
          (lambda () (screen-matches? machine "expected/pc98-title.png" #f))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (screen-matches? machine "expected/pc98-title.png"
              '((unknown . 1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (screen-matches? machine "expected/pc98-title.png"
              '((tolerance . 0.0) (tolerance . 1.0))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (screen-matches? machine "expected/pc98-title.png"
              '((tolerance . 2.0))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (screen-matches? machine "expected/pc98-title.png" 0.0))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (screen-matches? machine "expected/pc98-title.png" '() '()))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (screen-region-matches?
              machine "expected/pc98-title.png" 0 0 640 400 #f))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (screen-region-matches?
              machine "expected/pc98-title.png" 0 0 640 400
              '((unknown . 1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (screen-region-matches?
              machine "expected/pc98-title.png" 0 0 640 400
              '((tolerance . 0.0) (tolerance . 1.0))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (screen-region-matches?
              machine "expected/pc98-title.png" 0 0 640 400
              '((tolerance . 2.0))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (screen-region-matches?
              machine "expected/pc98-title.png" 0 0 640 400 0.0))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (screen-region-matches?
              machine "expected/pc98-title.png" 0 0 640 400 '() '()))))
      (check-true
        (raises? 'neetan/argument
          (lambda () (check-screen machine "expected/pc98-title.png" #f))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (check-screen machine "expected/pc98-title.png"
              '((unknown . 1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (check-screen machine "expected/pc98-title.png"
              '((tolerance . 0.0) (tolerance . 1.0))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (check-screen machine "expected/pc98-title.png"
              '((tolerance . 2.0))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (check-screen machine "expected/pc98-title.png" 0.0))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (check-screen machine "expected/pc98-title.png" '() '())))))

    (test-case "option validation uses the argument contract"
      (check-true
        (raises? 'neetan/argument
          (lambda () (wait-for-screen machine "expected/pc98-title.png" #f))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (wait-for-screen machine "expected/pc98-title.png"
              '((unknown . 1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (wait-for-screen machine "expected/pc98-title.png"
              '((frames . 1) (frames . 2))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (wait-for-screen machine "expected/pc98-title.png"
              '((tolerance . 2.0))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (wait-for-screen machine "expected/pc98-title.png"
              '((frames . -1))))))
      (check-true
        (raises? 'neetan/argument
          (lambda ()
            (wait-for-screen machine "expected/pc98-title.png" '() '())))))))
