// Configuration - Update URLs here
// Automatically detect base path from current location
const currentPath = window.location.pathname;
const basePath = currentPath.replace(/\/[^\/]*$/, ''); // Remove last segment (e.g., /index.html)

const CONFIG = {
    dashboardUrl: basePath ? `${basePath}/dashboard` : '/dashboard',
    githubUrl: 'https://github.com/your-username/streamvault',
    // Add other configurable URLs here as needed
};

// Update all dashboard links on page load
document.addEventListener('DOMContentLoaded', () => {
    // Update all dashboard links
    document.querySelectorAll('a[href="/dashboard"]').forEach(link => {
        link.href = CONFIG.dashboardUrl;
    });
    
    // Update GitHub links
    document.querySelectorAll('a[href="https://github.com/your-username/streamvault"]').forEach(link => {
        link.href = CONFIG.githubUrl;
    });
});

// Smooth scrolling for navigation links
document.querySelectorAll('a[href^="#"]').forEach(anchor => {
    anchor.addEventListener('click', function (e) {
        e.preventDefault();
        const target = document.querySelector(this.getAttribute('href'));
        if (target) {
            target.scrollIntoView({
                behavior: 'smooth',
                block: 'start'
            });
        }
    });
});

// Navbar scroll effect
window.addEventListener('scroll', () => {
    const navbar = document.querySelector('.navbar');
    if (window.scrollY > 100) {
        navbar.style.background = 'rgba(255, 255, 255, 0.98)';
        navbar.style.boxShadow = '0 4px 6px -1px rgb(0 0 0 / 0.1)';
    } else {
        navbar.style.background = 'rgba(255, 255, 255, 0.95)';
        navbar.style.boxShadow = 'none';
    }
});

// Mobile menu toggle
const hamburger = document.querySelector('.hamburger');
const navMenu = document.querySelector('.nav-menu');

hamburger?.addEventListener('click', () => {
    hamburger.classList.toggle('active');
    navMenu.classList.toggle('active');
});

// Close mobile menu when clicking on a link
document.querySelectorAll('.nav-link').forEach(link => {
    link.addEventListener('click', () => {
        hamburger?.classList.remove('active');
        navMenu?.classList.remove('active');
    });
});

// Intersection Observer for fade-in animations
const observerOptions = {
    threshold: 0.1,
    rootMargin: '0px 0px -50px 0px'
};

const observer = new IntersectionObserver((entries) => {
    entries.forEach(entry => {
        if (entry.isIntersecting) {
            entry.target.style.opacity = '1';
            entry.target.style.transform = 'translateY(0)';
        }
    });
}, observerOptions);

// Add fade-in animation to elements
document.addEventListener('DOMContentLoaded', () => {
    const animatedElements = document.querySelectorAll('.feature-card, .integration-item, .tech-item, .developer-card');
    
    animatedElements.forEach(el => {
        el.style.opacity = '0';
        el.style.transform = 'translateY(30px)';
        el.style.transition = 'opacity 0.6s ease, transform 0.6s ease';
        observer.observe(el);
    });
});

// Dashboard preview interaction
document.addEventListener('DOMContentLoaded', () => {
    const cameraFeeds = document.querySelectorAll('.camera-feed:not(.offline)');
    
    // Add subtle animation to camera feeds
    cameraFeeds.forEach((feed, index) => {
        setTimeout(() => {
            feed.style.animation = `stream 2s linear infinite ${index * 0.5}s`;
        }, index * 200);
    });
    
    // Camera tile click interaction
    const cameraTiles = document.querySelectorAll('.camera-tile');
    cameraTiles.forEach(tile => {
        tile.addEventListener('click', () => {
            // Remove active class from all tiles
            cameraTiles.forEach(t => t.classList.remove('active'));
            // Add active class to clicked tile
            tile.classList.add('active');
        });
    });
});

// Parallax effect for hero background
window.addEventListener('scroll', () => {
    const scrolled = window.pageYOffset;
    const heroBackground = document.querySelector('.hero-background');
    const heroParticles = document.querySelector('.hero-particles');
    
    if (heroBackground) {
        heroBackground.style.transform = `translateY(${scrolled * 0.3}px)`;
    }
    
    if (heroParticles) {
        heroParticles.style.transform = `translateY(${scrolled * 0.2}px)`;
    }
});

// Add typing animation to hero title (disabled for now)
document.addEventListener('DOMContentLoaded', () => {
    // Typing animation disabled to prevent HTML rendering issues
    // const heroTitle = document.querySelector('.hero-title');
    // Title will display normally without typing effect
});

// Add counter animation for stats
document.addEventListener('DOMContentLoaded', () => {
    const statsObserver = new IntersectionObserver((entries) => {
        entries.forEach(entry => {
            if (entry.isIntersecting) {
                const statNumbers = entry.target.querySelectorAll('.stat-number');
                statNumbers.forEach(stat => {
                    const text = stat.textContent;
                    if (text.includes('%')) {
                        // Animate percentage
                        const percentage = parseFloat(text);
                        animateNumber(stat, 0, percentage, 2000, '%');
                    }
                });
                statsObserver.unobserve(entry.target);
            }
        });
    }, { threshold: 0.5 });
    
    const heroStats = document.querySelector('.hero-stats');
    if (heroStats) {
        statsObserver.observe(heroStats);
    }
});

function animateNumber(element, start, end, duration, suffix = '') {
    const startTime = performance.now();
    
    const updateNumber = (currentTime) => {
        const elapsed = currentTime - startTime;
        const progress = Math.min(elapsed / duration, 1);
        
        // Easing function for smooth animation
        const easeOut = 1 - Math.pow(1 - progress, 3);
        const current = start + (end - start) * easeOut;
        
        element.textContent = current.toFixed(1) + suffix;
        
        if (progress < 1) {
            requestAnimationFrame(updateNumber);
        } else {
            element.textContent = end + suffix;
        }
    };
    
    requestAnimationFrame(updateNumber);
}

// Add smooth reveal animation for sections
const sectionObserver = new IntersectionObserver((entries) => {
    entries.forEach(entry => {
        if (entry.isIntersecting) {
            entry.target.classList.add('section-visible');
        }
    });
}, { threshold: 0.1 });

document.addEventListener('DOMContentLoaded', () => {
    const sections = document.querySelectorAll('section');
    sections.forEach(section => {
        section.style.opacity = '0';
        section.style.transform = 'translateY(20px)';
        section.style.transition = 'opacity 0.8s ease, transform 0.8s ease';
        sectionObserver.observe(section);
    });
});

// Add CSS for section visibility
const style = document.createElement('style');
style.textContent = `
    .section-visible {
        opacity: 1 !important;
        transform: translateY(0) !important;
    }
    
    @media (max-width: 768px) {
        .nav-menu {
            position: fixed;
            left: -100%;
            top: 70px;
            flex-direction: column;
            background-color: rgba(255, 255, 255, 0.98);
            width: 100%;
            text-align: center;
            transition: 0.3s;
            box-shadow: 0 10px 27px rgba(0, 0, 0, 0.05);
            backdrop-filter: blur(10px);
            padding: 20px 0;
        }
        
        .nav-menu.active {
            left: 0;
        }
        
        .hamburger.active span:nth-child(2) {
            opacity: 0;
        }
        
        .hamburger.active span:nth-child(1) {
            transform: translateY(9px) rotate(45deg);
        }
        
        .hamburger.active span:nth-child(3) {
            transform: translateY(-9px) rotate(-45deg);
        }
    }
`;
document.head.appendChild(style);

// Add SCADA diagram animation
document.addEventListener('DOMContentLoaded', () => {
    const scadaDiagram = document.querySelector('.scada-diagram');
    if (scadaDiagram) {
        const dataFlows = scadaDiagram.querySelectorAll('.data-flow');
        
        // Animate data flows
        dataFlows.forEach((flow, index) => {
            flow.style.opacity = '0';
            flow.style.animation = `fadeInPulse 1.5s ease-in-out ${index * 0.5}s infinite`;
        });
    }
});

// Add CSS for data flow animation
const dataFlowStyle = document.createElement('style');
dataFlowStyle.textContent = `
    @keyframes fadeInPulse {
        0%, 100% { opacity: 0.7; transform: scale(1); }
        50% { opacity: 1; transform: scale(1.1); }
    }
`;
document.head.appendChild(dataFlowStyle);

// Add loading indicator for external links
document.addEventListener('DOMContentLoaded', () => {
    const externalLinks = document.querySelectorAll(`a[href^="http"], a[href="${CONFIG.dashboardUrl}"]`);
    
    externalLinks.forEach(link => {
        link.addEventListener('click', (e) => {
            const btn = e.target.closest('.btn-primary, .btn-secondary, .nav-button');
            if (btn) {
                const originalText = btn.innerHTML;
                btn.innerHTML = '<i class="fas fa-spinner fa-spin"></i> Loading...';
                btn.style.pointerEvents = 'none';
                
                // Reset after 3 seconds if page hasn't changed
                setTimeout(() => {
                    if (btn) {
                        btn.innerHTML = originalText;
                        btn.style.pointerEvents = 'auto';
                    }
                }, 3000);
            }
        });
    });
});