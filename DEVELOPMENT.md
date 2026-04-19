# Development Notes

## Removing broken release/tag

```
# Delete tag locally and remotely
git tag -d v<ver>
git push origin :refs/tags/v<ver>

# Re-create and push the tag at the current HEAD
git tag -a v<ver> -m "Release <ver>"
git push origin v<ver>
```
```
```

