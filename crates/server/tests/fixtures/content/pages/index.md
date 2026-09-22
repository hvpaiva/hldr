{{ banner site.values.banner }}

# {{ profile.name }}
> {{ profile.headline }}

{{ profile.bio }}

## {{ link /projects Projects }}
{{ collection projects where=highlight limit=3 }}

## About
{{ site.values.about }} More in {{ link /about about.md }}.

## Elsewhere
{{ links source }}
