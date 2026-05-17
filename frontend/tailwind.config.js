/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        kotonoha: {
          paper:  "#FDF7EE",
          ink:    "#2B2A28",
          accent: "#E07A5F",
          leaf:   "#7BAE8A",
        },
      },
      fontFamily: {
        ja: ["Shippori Mincho", "Noto Sans JP", "serif"],
        en: ["Inter", "system-ui", "sans-serif"],
      },
    },
  },
  plugins: [],
};
