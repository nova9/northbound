const carousel = document.querySelector('.screenshot-carousel');
const track = carousel.querySelector('.screenshot-track');
const slides = Array.from(track.querySelectorAll('.screenshot-slide'));
const controls = carousel.querySelector('.carousel-controls');
const picker = carousel.querySelector('.carousel-picker');
const slideButtons = Array.from(picker.querySelectorAll('button'));
const status = carousel.querySelector('.carousel-status');
const autoplayButton = carousel.querySelector('.carousel-autoplay');
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
const slideDurationMilliseconds = 5000;
let currentSlide = 0;
let autoplayPaused = reducedMotion.matches;
let autoplayTimer;

function scheduleNextSlide() {
  window.clearTimeout(autoplayTimer);
  const shouldPause = autoplayPaused || document.hidden
    || carousel.matches(':hover') || carousel.contains(document.activeElement);
  status.setAttribute('aria-live', shouldPause ? 'polite' : 'off');
  autoplayButton.textContent = autoplayPaused ? 'Play' : 'Pause';
  autoplayButton.setAttribute('aria-label', autoplayPaused ? 'Start automatic slideshow' : 'Pause automatic slideshow');

  if (shouldPause) return;

  autoplayTimer = window.setTimeout(() => {
    scrollToSlide(currentSlide + 1);
    scheduleNextSlide();
  }, slideDurationMilliseconds);
}

function scrollToSlide(index) {
  const nextSlide = (index + slides.length) % slides.length;
  track.scrollTo({
    left: nextSlide * track.clientWidth,
    behavior: reducedMotion.matches ? 'instant' : 'smooth',
  });
}

carousel.querySelector('.carousel-previous').addEventListener('click', () => {
  scrollToSlide(currentSlide - 1);
});

carousel.querySelector('.carousel-next').addEventListener('click', () => {
  scrollToSlide(currentSlide + 1);
});

for (const button of slideButtons) {
  button.addEventListener('click', () => {
    scrollToSlide(Number(button.dataset.slide));
  });
}

track.addEventListener('keydown', (event) => {
  if (event.key === 'ArrowLeft') {
    event.preventDefault();
    scrollToSlide(currentSlide - 1);
  } else if (event.key === 'ArrowRight') {
    event.preventDefault();
    scrollToSlide(currentSlide + 1);
  }
});

track.addEventListener('scroll', () => {
  const nearestSlide = Math.round(track.scrollLeft / track.clientWidth);
  const visibleSlide = Math.max(0, Math.min(nearestSlide, slides.length - 1));
  if (visibleSlide === currentSlide) return;

  currentSlide = visibleSlide;
  status.textContent = `${slides[currentSlide].dataset.title} · ${currentSlide + 1} / ${slides.length}`;
  for (const button of slideButtons) {
    button.setAttribute('aria-pressed', String(Number(button.dataset.slide) === currentSlide));
  }
}, { passive: true });

autoplayButton.addEventListener('click', () => {
  autoplayPaused = !autoplayPaused;
  scheduleNextSlide();
});

carousel.addEventListener('mouseenter', scheduleNextSlide);
carousel.addEventListener('mouseleave', scheduleNextSlide);
carousel.addEventListener('focusin', scheduleNextSlide);
carousel.addEventListener('focusout', () => {
  // Wait until the browser has moved focus to its next destination.
  window.setTimeout(scheduleNextSlide, 0);
});
carousel.addEventListener('touchstart', () => window.clearTimeout(autoplayTimer), { passive: true });
carousel.addEventListener('touchend', scheduleNextSlide, { passive: true });
carousel.addEventListener('touchcancel', scheduleNextSlide, { passive: true });
document.addEventListener('visibilitychange', scheduleNextSlide);
reducedMotion.addEventListener('change', () => {
  autoplayPaused = reducedMotion.matches;
  scheduleNextSlide();
});

controls.hidden = false;
picker.hidden = false;
scheduleNextSlide();
