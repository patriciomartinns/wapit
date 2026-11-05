document.documentElement.classList.add('js');
try {
  const theme = localStorage.getItem('wapit.theme');
  if (theme === 'light' || theme === 'dark') document.documentElement.dataset.theme = theme;
} catch {}
