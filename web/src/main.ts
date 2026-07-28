import './app.css';
// imported for effect: applies the resolved theme and keeps following the system
import './lib/theme.svelte';
import { mount } from 'svelte';
import App from './App.svelte';

export default mount(App, { target: document.getElementById('app')! });
