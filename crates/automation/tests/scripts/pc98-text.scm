;; PC-98 decoded text-surface inspection through the public (neetan inspect 1)
;; library. Boots the HLE machine to its "Neetan DOS" banner and reads the
;; decoded text surface: geometry, per-cell Unicode and raw attribute, whole
;; screen rows, and wait-for-text stopping at the first match.
(import (scheme base)
        (neetan automation 1)
        (neetan inspect 1)
        (neetan test 1))

;; Returns the value for key in an alist, or #f.
(define (field key alist)
  (let ((entry (assq key alist)))
    (and entry (cdr entry))))

;; Returns #t when the first string contains the second.
(define (string-contains? haystack needle)
  (let ((hn (string-length haystack)) (nn (string-length needle)))
    (let loop ((start 0))
      (cond ((> (+ start nn) hn) #f)
            ((string=? (substring haystack start (+ start nn)) needle) #t)
            (else (loop (+ start 1)))))))

(test-suite "PC-98 text surface"
  (with-machine (machine '((target . pc98) (model . pc9801vm)))
    (test-case "surface discovery and geometry"
      (check-equal '(display.main) (text-surfaces machine))
      (let ((info (text-surface-info machine 'display.main)))
        (check-equal 'display.main (field 'id info))
        (check-equal 25 (field 'rows info))
        (check-equal 80 (field 'columns info))))

    (test-case "decoded cell exposes Unicode and the raw attribute"
      (run-frames! machine 30)
      ;; The banner begins with 'N' at the top-left cell.
      (let ((cell (text-cell machine 'display.main 0 0)))
        (check-equal 0 (field 'row cell))
        (check-equal 0 (field 'column cell))
        (check-equal 78 (field 'raw-jis cell))
        (check-equal #\N (field 'unicode cell))
        (check-equal 1 (field 'display-width cell))
        (check-true (exact? (field 'attribute cell)))))

    (test-case "decoded screen rows read left to right"
      (let ((rows (text-screen machine 'display.main)))
        (check-true (pair? rows))
        (check-equal 25 (length rows))
        (check-true (string-contains? (car rows) "Neetan DOS"))))

    (test-case "wait-for-text stops at the first match"
      ;; The banner is already present, so the wait returns the matched text.
      (let ((matched (wait-for-text machine 'display.main
                       '((row . 0) (contains . "Neetan DOS")))))
        (check-true (string? matched))
        (check-true (string-contains? matched "Neetan DOS")))
      ;; A bare string is shorthand for ((contains . string)).
      (let ((matched (wait-for-text machine 'display.main "Neetan DOS")))
        (check-true (string? matched))
        (check-true (string-contains? matched "Neetan DOS")))
      ;; A substring that never appears exhausts the small frame bound and
      ;; returns #f.
      (check-false (wait-for-text machine 'display.main
                     '((contains . "no-such-text-anywhere"))
                     '((frames . 2)))))

    (test-case "text errors follow the argument contract"
      ;; An unknown surface is an argument error.
      (check-true
        (guard (condition (#t (and (error-object? condition)
                                   (memq 'neetan/argument
                                         (error-object-irritants condition))
                                   #t)))
          (text-surface-info machine 'display.other)
          #f))
      ;; An out-of-range cell is a range error.
      (check-true
        (guard (condition (#t (and (error-object? condition)
                                   (memq 'neetan/range
                                         (error-object-irritants condition))
                                   #t)))
          (text-cell machine 'display.main 99 0)
          #f)))))
