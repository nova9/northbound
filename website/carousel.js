const carousel = document.querySelector('.screenshot-carousel');
const track = carousel.querySelector('.screenshot-track');
const slides = Array.from(track.querySelectorAll('.screenshot-slide'));
const picker = carousel.querySelector('.carousel-picker');
const slideButtons = Array.from(picker.querySelectorAll('button'));
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
const slideDurationMilliseconds = 5000;
let currentSlide = 0;
let autoplayTimer;

carousel.style.setProperty('--slide-duration', `${slideDurationMilliseconds}ms`);

function stopCountdown() {
  window.clearTimeout(autoplayTimer);
  carousel.classList.remove('is-counting');
}

function scheduleNextSlide() {
  stopCountdown();
  const shouldPause = reducedMotion.matches || document.hidden
    || carousel.matches(':hover') || carousel.contains(document.activeElement);

  if (shouldPause) return;

  // Force a reflow so the countdown bar restarts from full width.
  void carousel.offsetWidth;
  carousel.classList.add('is-counting');

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
  for (const button of slideButtons) {
    button.setAttribute('aria-pressed', String(Number(button.dataset.slide) === currentSlide));
  }
  scheduleNextSlide();
}, { passive: true });

carousel.addEventListener('mouseenter', scheduleNextSlide);
carousel.addEventListener('mouseleave', scheduleNextSlide);
carousel.addEventListener('focusin', scheduleNextSlide);
carousel.addEventListener('focusout', () => {
  // Wait until the browser has moved focus to its next destination.
  window.setTimeout(scheduleNextSlide, 0);
});
carousel.addEventListener('touchstart', stopCountdown, { passive: true });
carousel.addEventListener('touchend', scheduleNextSlide, { passive: true });
carousel.addEventListener('touchcancel', scheduleNextSlide, { passive: true });
document.addEventListener('visibilitychange', scheduleNextSlide);
reducedMotion.addEventListener('change', scheduleNextSlide);

picker.hidden = false;
scheduleNextSlide();
