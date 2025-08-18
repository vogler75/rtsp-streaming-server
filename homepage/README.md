# StreamVault Homepage

This directory contains the modern, professional homepage for StreamVault.

## Features

- **Modern Design**: Clean, professional layout with gradient backgrounds and smooth animations
- **Responsive**: Fully responsive design that works on desktop, tablet, and mobile
- **Interactive Elements**: 
  - Animated dashboard preview
  - SCADA integration diagram
  - Smooth scrolling navigation
  - Parallax effects
  - Fade-in animations
- **Professional Presentation**: Showcases all StreamVault features and capabilities
- **SCADA Integration**: Dedicated section highlighting WinCC Unified integration

## Files

- `index.html` - Main homepage HTML structure
- `styles.css` - Complete CSS styling with modern design system
- `script.js` - Interactive JavaScript functionality
- `README.md` - This documentation file

## Integration with StreamVault

To integrate this homepage with your StreamVault server, you can:

1. **Serve as static files**: Place these files in a `static/homepage/` directory and serve them
2. **Custom route**: Add a route handler to serve `index.html` at the root path `/`
3. **Reverse proxy**: Use nginx or similar to serve the homepage alongside the main application

## Customization

The homepage includes:

- Your name "Andreas Vogler" prominently featured
- Detailed feature descriptions
- WinCC Unified and Comfort Panel integration information
- Professional branding and messaging
- Call-to-action buttons linking to the dashboard

## Design System

The CSS uses modern design principles:

- CSS custom properties for consistent theming
- CSS Grid and Flexbox for responsive layouts
- Smooth transitions and animations
- Professional color palette
- Modern typography (Inter font)

## Browser Support

Compatible with all modern browsers supporting:
- CSS Grid
- CSS Custom Properties
- ES6 JavaScript
- Intersection Observer API

The design gracefully degrades for older browsers while maintaining functionality.